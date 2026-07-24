// arch-exempt: large_file, channel-neutral lifecycle-event and trigger-target regressions reuse the shared delivery harness, plan #4088
//! Contract rows for the generic run-delivery components (§5.4, 9b): the
//! live observer and the triggered driver, driven with scripted run states
//! and a scripted channel adapter, asserting at the coordinator/store seam.
//! The channel-level regression net (the vendor e2e scenarios through the
//! real ingress mount) re-points onto these components at the cutover.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_outbound::{
    CommunicationModality, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    DeliveredGateRouteStore, DeliveryDefaultScope, FilesystemOutboundStateStore,
    OutboundStateStore, RunDeliveryCleanupRequest, RunFinalReplyDestination,
    RunFinalReplyTargetRecord, TriggerCommunicationContext, TriggerFireSlot, TriggerOriginRef,
    TriggerSourceKind, TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryStore,
};
use ironclaw_product::{
    AdapterInstallationId, AuthPromptChallengeKind, AuthPromptView, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ChannelAdapter, ChannelError, DeliveryReport,
    ExternalActorRef, ExternalConversationRef, ExternalEventId, InboundOutcome, OutboundEnvelope,
    OutboundPart, PairingPromptView, ParsedProductInbound, PartDeliveryOutcome,
    ProductAdapterError, ProductAdapterId, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductRejection, ProductRejectionKind, ProductTriggerReason,
    ProtocolAuthEvidence, TrustedInboundContext, UserMessagePayload, VerifiedInbound,
    render_channel_auth_prompt,
};
use ironclaw_product::{
    BlockedAuthPromptRequest, BlockedAuthPromptSource, ChannelConnectionNoticePolicy,
    ChannelDeliveryResolver, CurrentDeliveryTarget, CurrentDeliveryTargetResolver,
    DeliveryCoordinator, DeliveryReplyContextSource, DeliveryRetryPolicy,
    ProductConversationRouteKind, ProductWorkflowError, ResolveBindingRequest,
    ResolveStoredProductReplyTargetRequest, ResolvedChannelDelivery,
    ResolvedStoredProductReplyTarget, RunDeliveryEventHandler, RunDeliveryEventRouter,
    RunDeliveryObserver, RunDeliveryServices, TriggeredRunDeliveryDriver,
    TriggeredRunDeliveryRequest, TriggeredRunExternalDeliveryTarget,
};
use ironclaw_threads::{
    AppendFinalizedAssistantMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
    MessageContent, SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, ProductTurnContext, ReplyTargetBindingRef, ResumeTurnRequest,
    ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse, RunOriginAdapter, RunProfileId,
    RunProfileResolver, RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnAdmissionPolicy, TurnCoordinator, TurnError, TurnEventKind, TurnEventPage,
    TurnEventProjectionSource, TurnEventSink, TurnId, TurnLifecycleEvent, TurnOriginKind,
    TurnOwner, TurnRunId, TurnRunState, TurnScope, TurnStateStore, TurnStatus, TurnSurfaceType,
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
                    OutboundPart::AuthPrompt {
                        view,
                        direct_message,
                    } => Some(render_channel_auth_prompt(view, *direct_message)),
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

struct StaticBindingService {
    binding: ironclaw_product::ResolvedBinding,
    fail: bool,
}

#[async_trait]
impl ironclaw_product::ConversationBindingService for StaticBindingService {
    async fn resolve_binding(
        &self,
        _request: ironclaw_product::ResolveBindingRequest,
    ) -> Result<ironclaw_product::ResolvedBinding, ironclaw_product::ProductWorkflowError> {
        if self.fail {
            return Err(
                ironclaw_product::ProductWorkflowError::BindingResolutionFailed {
                    reason: "unbound".to_string(),
                },
            );
        }
        Ok(self.binding.clone())
    }

    async fn lookup_binding(
        &self,
        _request: ironclaw_product::ResolveBindingRequest,
    ) -> Result<ironclaw_product::ResolvedBinding, ironclaw_product::ProductWorkflowError> {
        if self.fail {
            return Err(
                ironclaw_product::ProductWorkflowError::BindingResolutionFailed {
                    reason: "unbound".to_string(),
                },
            );
        }
        Ok(self.binding.clone())
    }
}

struct StaticAuthPromptSource {
    challenge_kind: AuthPromptChallengeKind,
    authorization_url: Option<String>,
    pairing: Option<PairingPromptView>,
    body_override: Option<String>,
}

#[async_trait]
impl BlockedAuthPromptSource for StaticAuthPromptSource {
    async fn auth_prompt_for_blocked_run(
        &self,
        request: BlockedAuthPromptRequest<'_>,
    ) -> Result<AuthPromptView, ProductAdapterError> {
        Ok(AuthPromptView {
            turn_run_id: request.run_id,
            auth_request_ref: request.gate_ref.to_string(),
            invocation_id: None,
            headline: "Authentication required".to_string(),
            body: self.body_override.clone().unwrap_or(request.body),
            challenge_kind: Some(self.challenge_kind),
            provider: None,
            account_label: None,
            authorization_url: self.authorization_url.clone(),
            expires_at: None,
            connection: None,
            pairing: self.pairing.clone(),
        })
    }
}

struct StaticTriggeredTargetResolver {
    extension_id: String,
    conversation: ExternalConversationRef,
    personal_dm: bool,
    available: AtomicBool,
}

impl StaticTriggeredTargetResolver {
    fn revoke(&self) {
        self.available.store(false, Ordering::SeqCst);
    }
}

#[async_trait]
impl CurrentDeliveryTargetResolver for StaticTriggeredTargetResolver {
    async fn resolve_current_target(
        &self,
        _scope: &TurnScope,
        _actor: &TurnActor,
        _target: &ReplyTargetBindingRef,
    ) -> Result<Option<CurrentDeliveryTarget>, ProductWorkflowError> {
        if !self.available.load(Ordering::SeqCst) {
            return Ok(None);
        }
        Ok(Some(CurrentDeliveryTarget {
            extension_id: self.extension_id.clone(),
            external_conversation_ref: self.conversation.clone(),
            personal_direct_message: self.personal_dm,
        }))
    }

    async fn resolve_current_destination(
        &self,
        _scope: &ironclaw_host_api::ResourceScope,
        _target_id: &ironclaw_outbound::OutboundDeliveryTargetId,
    ) -> Result<Option<ironclaw_outbound::RunFinalReplyDestination>, ProductWorkflowError> {
        Ok(None)
    }

    async fn resolve_current_target_id(
        &self,
        _scope: &ironclaw_host_api::ResourceScope,
        _target: &ReplyTargetBindingRef,
    ) -> Result<Option<ironclaw_outbound::OutboundDeliveryTargetId>, ProductWorkflowError> {
        Ok(None)
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

fn binding() -> ironclaw_product::ResolvedBinding {
    ironclaw_product::ResolvedBinding {
        tenant_id: tenant(),
        actor_user_id: user(),
        subject_user_id: Some(user()),
        source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:test-binding")
            .expect("source binding ref"),
        reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new("reply:test-binding")
            .expect("reply target binding ref"),
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
}

#[allow(clippy::too_many_arguments)]
fn build_harness(
    states: Vec<ScriptedRunState>,
    bind_fails: bool,
    auth_url: Option<&str>,
) -> Harness {
    build_harness_with_binding(states, bind_fails, auth_url, binding())
}

fn build_harness_with_binding(
    states: Vec<ScriptedRunState>,
    bind_fails: bool,
    auth_url: Option<&str>,
    binding: ironclaw_product::ResolvedBinding,
) -> Harness {
    let blocked_auth_prompts = auth_url.map(|url| {
        Arc::new(StaticAuthPromptSource {
            challenge_kind: AuthPromptChallengeKind::OAuthUrl,
            authorization_url: Some(url.to_string()),
            pairing: None,
            body_override: None,
        }) as Arc<dyn BlockedAuthPromptSource>
    });
    build_harness_with_prompt(states, bind_fails, blocked_auth_prompts, binding)
}

fn build_harness_with_prompt(
    states: Vec<ScriptedRunState>,
    bind_fails: bool,
    blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    binding: ironclaw_product::ResolvedBinding,
) -> Harness {
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let route_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let turns = Arc::new(ScriptedTurnCoordinator::with_states(states));
    let threads = Arc::new(InMemorySessionThreadService::default());
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
            binding,
            fail: bind_fails,
        }),
        thread_service: Arc::clone(&threads) as Arc<dyn SessionThreadService>,
        turn_coordinator: Arc::clone(&turns) as Arc<dyn TurnCoordinator>,
        outbound_store: Arc::clone(&store) as Arc<dyn OutboundStateStore>,
        route_store: Arc::clone(&route_store) as Arc<dyn DeliveredGateRouteStore>,
        communication_preferences: Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>,
        coordinator,
        extension_id: EXTENSION_ID.to_string(),
        fallback_notice_scope: fallback_scope(),
        approval_context: None,
        blocked_auth_prompts,
        auth_flow_cancel: None,
    };
    let connection_notices = ChannelConnectionNoticePolicy::generic("Acme");
    let observer = Arc::new(RunDeliveryObserver::with_connection_notices(
        services,
        connection_notices.clone(),
    ));
    Harness {
        observer,
        connection_notices,
        adapter,
        store,
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
async fn observer_accepted_auth_denial_does_not_send_notice_when_binding_lookup_fails() {
    let harness = build_harness(vec![scripted_state(TurnStatus::Running, None)], true, None);
    let envelope = envelope(
        ProductInboundPayload::AuthResolution(
            AuthResolutionPayload::new("gate:auth-denial", AuthResolutionResult::Denied)
                .expect("auth resolution"),
        ),
        "evt-auth-denial",
    );

    harness
        .observer
        .observe_ack(envelope, accepted_ack(TurnRunId::new()))
        .await;

    assert!(
        harness.adapter.envelopes().is_empty(),
        "a failed binding lookup must not fall back to an unrelated authority scope"
    );
}

#[tokio::test]
async fn observer_accepted_auth_denial_does_not_send_notice_when_binding_scope_is_invalid() {
    let mut invalid_binding = binding();
    invalid_binding.agent_id = None;
    let harness = build_harness_with_binding(
        vec![scripted_state(TurnStatus::Running, None)],
        false,
        None,
        invalid_binding,
    );
    let envelope = envelope(
        ProductInboundPayload::AuthResolution(
            AuthResolutionPayload::new("gate:auth-denial", AuthResolutionResult::Denied)
                .expect("auth resolution"),
        ),
        "evt-auth-denial-invalid-scope",
    );

    harness
        .observer
        .observe_ack(envelope, accepted_ack(TurnRunId::new()))
        .await;

    assert!(
        harness.adapter.envelopes().is_empty(),
        "an invalid binding scope must not fall back to an unrelated authority scope"
    );
}

#[tokio::test]
async fn observer_connect_nudge_posts_only_for_direct_chat_binding_required() {
    let harness = build_harness(vec![scripted_state(TurnStatus::Running, None)], true, None);
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
    let harness = build_harness(vec![scripted_state(TurnStatus::Running, None)], true, None);
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

    let harness = build_harness(vec![scripted_state(TurnStatus::Running, None)], true, None);
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
    let harness = build_harness(vec![scripted_state(TurnStatus::Running, None)], false, None);
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
async fn observer_busy_hint_reprojects_oauth_challenge_for_direct_channel() {
    let harness = build_harness(
        vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("auth:busy-oauth"),
        )],
        false,
        Some("https://provider.example/oauth"),
    );
    let active_run = TurnRunId::new();

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-busy-oauth"),
            ProductInboundAck::RejectedBusy {
                accepted_message_ref: AcceptedMessageRef::new("msg:busy-oauth").expect("ref"),
                active_run_id: Some(active_run),
            },
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1);
    assert!(
        texts[0].contains("Setup link: https://provider.example/oauth"),
        "the typed OAuth challenge must remain actionable: {}",
        texts[0]
    );
    assert!(texts[0].contains("auth deny auth:busy-oauth"));
}

#[tokio::test]
async fn observer_busy_hint_reprojects_pairing_code_and_deep_link_for_direct_channel() {
    let harness = build_harness_with_prompt(
        vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("auth:busy-pairing"),
        )],
        false,
        Some(Arc::new(StaticAuthPromptSource {
            challenge_kind: AuthPromptChallengeKind::Pairing,
            authorization_url: None,
            pairing: Some(PairingPromptView {
                channel: "fixture".to_string(),
                display_name: "Fixture Chat".to_string(),
                instructions: "Open the link or send `/start <code>` to the configured bot."
                    .to_string(),
                code: "BUSY42AB".to_string(),
                deep_link: Some("https://example.test/start=BUSY42AB".to_string()),
                expires_at: chrono::DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
                    .expect("expiry")
                    .with_timezone(&Utc),
            }),
            body_override: None,
        })),
        binding(),
    );
    let active_run = TurnRunId::new();

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-busy-pairing"),
            ProductInboundAck::RejectedBusy {
                accepted_message_ref: AcceptedMessageRef::new("msg:busy-pairing").expect("ref"),
                active_run_id: Some(active_run),
            },
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1);
    assert!(
        texts[0].contains("BUSY42AB"),
        "missing pairing code: {}",
        texts[0]
    );
    assert!(
        texts[0].contains("https://example.test/start=BUSY42AB"),
        "missing pairing deep link: {}",
        texts[0]
    );
    assert!(texts[0].contains("`/start <code>`"));
}

#[tokio::test]
async fn observer_busy_hint_does_not_expose_auth_authority_in_shared_channel() {
    let harness = build_harness(
        vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("auth:busy-shared"),
        )],
        false,
        Some("https://provider.example/private-oauth"),
    );
    let active_run = TurnRunId::new();

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::BotMention, "evt-busy-shared"),
            ProductInboundAck::RejectedBusy {
                accepted_message_ref: AcceptedMessageRef::new("msg:busy-shared").expect("ref"),
                active_run_id: Some(active_run),
            },
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("private authorization step"));
    assert!(!texts[0].contains("private-oauth"));
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

#[test]
fn triggered_external_delivery_target_preserves_typed_destination_semantics() {
    assert_eq!(
        TriggeredRunExternalDeliveryTarget::from_destination(None),
        Some(TriggeredRunExternalDeliveryTarget::UseCommunicationPreference),
        "an absent per-trigger target delegates to the creator's communication preference"
    );

    let target =
        ReplyTargetBindingRef::new("reply:test:explicit-trigger-target").expect("reply target");
    assert_eq!(
        TriggeredRunExternalDeliveryTarget::from_destination(Some(
            RunFinalReplyDestination::External {
                reply_target_binding_ref: target.clone(),
            },
        )),
        Some(TriggeredRunExternalDeliveryTarget::Explicit {
            reply_target_binding_ref: target,
        }),
        "the exact sealed external binding reaches channel routing"
    );

    assert_eq!(
        TriggeredRunExternalDeliveryTarget::from_destination(Some(
            RunFinalReplyDestination::WebApp,
        )),
        None,
        "WebApp history is not an external channel-delivery request"
    );
}

struct TriggeredHarness {
    driver: TriggeredRunDeliveryDriver,
    adapter: Arc<RecordingChannelAdapter>,
    store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    delivery_store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    turns: Arc<ScriptedTurnCoordinator>,
    threads: Arc<InMemorySessionThreadService>,
}

fn build_triggered_harness(
    states: Vec<ScriptedRunState>,
    auth_url: Option<&str>,
    personal_dm_target: bool,
) -> TriggeredHarness {
    build_triggered_harness_with_prompt(
        states,
        auth_url.map(|url| {
            Arc::new(StaticAuthPromptSource {
                challenge_kind: AuthPromptChallengeKind::OAuthUrl,
                authorization_url: Some(url.to_string()),
                pairing: None,
                body_override: None,
            }) as Arc<dyn BlockedAuthPromptSource>
        }),
        personal_dm_target,
    )
}

fn build_triggered_harness_with_prompt(
    states: Vec<ScriptedRunState>,
    blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
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
        coordinator,
        extension_id: EXTENSION_ID.to_string(),
        fallback_notice_scope: fallback_scope(),
        approval_context: None,
        blocked_auth_prompts,
        auth_flow_cancel: None,
    };
    let target_resolver = Arc::new(StaticTriggeredTargetResolver {
        extension_id: EXTENSION_ID.to_string(),
        conversation: ExternalConversationRef::new(Some("space-1"), "dm-creator", None, None)
            .expect("conversation"),
        personal_dm: personal_dm_target,
        available: AtomicBool::new(true),
    });
    let driver = TriggeredRunDeliveryDriver::with_event_router(
        services,
        Arc::clone(&delivery_store) as Arc<dyn TriggeredRunDeliveryStore>,
        Arc::clone(&target_resolver) as Arc<dyn CurrentDeliveryTargetResolver>,
        agent(),
        Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test()),
    );
    TriggeredHarness {
        driver,
        adapter,
        store,
        delivery_store,
        turns,
        threads,
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

// ── Durable lifecycle-event delivery rows ─────────────────────────────────

struct EventTurnCoordinator {
    state: Mutex<TurnRunState>,
    cancel_calls: AtomicUsize,
    fail_next_get_run_state: AtomicBool,
}

impl EventTurnCoordinator {
    fn new(state: TurnRunState) -> Self {
        Self {
            state: Mutex::new(state),
            cancel_calls: AtomicUsize::new(0),
            fail_next_get_run_state: AtomicBool::new(false),
        }
    }

    fn transition(&self, status: TurnStatus, gate_ref: Option<&str>, cursor: u64) -> TurnRunState {
        let mut state = self.state.lock().expect("event state");
        state.status = status;
        state.gate_ref = gate_ref.map(|value| GateRef::new(value).expect("gate"));
        state.event_cursor = EventCursor(cursor);
        state.clone()
    }

    fn cancel_call_count(&self) -> usize {
        self.cancel_calls.load(Ordering::SeqCst)
    }

    fn fail_next_get_run_state(&self) {
        self.fail_next_get_run_state.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl TurnCoordinator for EventTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "event fixture does not submit".into(),
        })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "event fixture does not resume".into(),
        })
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "event fixture does not retry".into(),
        })
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        self.cancel_calls.fetch_add(1, Ordering::SeqCst);
        let state = self.transition(TurnStatus::Cancelled, None, 99);
        Ok(CancelRunResponse {
            run_id: request.run_id,
            status: state.status,
            event_cursor: state.event_cursor,
            already_terminal: false,
            actor: state.actor,
        })
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        if self.fail_next_get_run_state.swap(false, Ordering::SeqCst) {
            return Err(TurnError::Unavailable {
                reason: "event fixture state is transiently unavailable".to_string(),
            });
        }
        Ok(self.state.lock().expect("event state").clone())
    }
}

struct RunMapTurnCoordinator {
    states: HashMap<TurnRunId, TurnRunState>,
}

#[async_trait]
impl TurnCoordinator for RunMapTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Err(TurnError::Unavailable {
            reason: "run-map fixture does not prepare turns".to_string(),
        })
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "run-map fixture does not submit turns".to_string(),
        })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "run-map fixture does not resume turns".to_string(),
        })
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "run-map fixture does not retry turns".to_string(),
        })
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "run-map fixture does not cancel turns".to_string(),
        })
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.states
            .get(&request.run_id)
            .filter(|state| state.scope == request.scope)
            .cloned()
            .ok_or(TurnError::ScopeNotFound)
    }
}

struct FailNextGetRunStateCoordinator {
    inner: Arc<EventTurnCoordinator>,
    fail_next: AtomicBool,
}

impl FailNextGetRunStateCoordinator {
    fn new(inner: Arc<EventTurnCoordinator>) -> Self {
        Self {
            inner,
            fail_next: AtomicBool::new(false),
        }
    }

    fn fail_next_get_run_state(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl TurnCoordinator for FailNextGetRunStateCoordinator {
    async fn prepare_turn(&self, scope: TurnScope) -> Result<TurnRunId, TurnError> {
        self.inner.prepare_turn(scope).await
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        self.inner.submit_turn(request).await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.inner.resume_turn(request).await
    }

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        self.inner.retry_turn(request).await
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        self.inner.cancel_run(request).await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(TurnError::Unavailable {
                reason: "source owner is transiently unavailable".to_string(),
            });
        }
        self.inner.get_run_state(request).await
    }
}

/// Adapts a scripted `TurnCoordinator` into the `TurnStateStore` seam the
/// production `RunDeliveryEventRouter::new` uses to classify a completed run's
/// origin/destination at materialization time. Only `get_run_state` is
/// exercised by the durable replay; the mutating turn operations are
/// unreachable from that path.
struct CoordinatorRunStateStore(Arc<dyn TurnCoordinator>);

#[async_trait]
impl TurnStateStore for CoordinatorRunStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "run-state store adapter does not submit turns".to_string(),
        })
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "run-state store adapter does not resume turns".to_string(),
        })
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "run-state store adapter does not retry turns".to_string(),
        })
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        Err(TurnError::Unavailable {
            reason: "run-state store adapter does not cancel turns".to_string(),
        })
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.0.get_run_state(request).await
    }
}

fn run_state_store(coordinator: Arc<dyn TurnCoordinator>) -> Arc<dyn TurnStateStore> {
    Arc::new(CoordinatorRunStateStore(coordinator))
}

struct StaticDurableTurnEventLog {
    events: Mutex<Vec<TurnLifecycleEvent>>,
    rebase_required: Mutex<Option<EventCursor>>,
}

impl StaticDurableTurnEventLog {
    fn new(events: Vec<TurnLifecycleEvent>) -> Self {
        Self {
            events: Mutex::new(events),
            rebase_required: Mutex::new(None),
        }
    }

    fn requiring_rebase(earliest: EventCursor) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            rebase_required: Mutex::new(Some(earliest)),
        }
    }
}

#[async_trait]
impl TurnEventProjectionSource for StaticDurableTurnEventLog {
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let mut page = self.read_turn_event_log_after(after, usize::MAX).await?;
        page.entries.retain(|event| {
            &event.scope == scope
                && owner_user_id.is_none_or(|owner| event.owner_user_id.as_ref() == Some(owner))
        });
        page.truncated = page.entries.len() > limit;
        if page.truncated {
            page.entries.truncate(limit);
        }
        page.next_cursor = page
            .entries
            .last()
            .map_or(after.unwrap_or_default(), |event| event.cursor);
        Ok(page)
    }

    async fn read_turn_event_log_after(
        &self,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        if let Some(earliest) = *self.rebase_required.lock().expect("rebase") {
            return Ok(TurnEventPage {
                entries: Vec::new(),
                next_cursor: earliest,
                truncated: false,
                rebase_required: Some(earliest),
            });
        }
        let after = after.unwrap_or_default();
        let mut entries = self
            .events
            .lock()
            .expect("events")
            .iter()
            .filter(|event| event.cursor > after)
            .cloned()
            .collect::<Vec<_>>();
        entries.sort_by_key(|event| event.cursor);
        let truncated = entries.len() > limit;
        entries.truncate(limit);
        let next_cursor = entries.last().map(|event| event.cursor).unwrap_or(after);
        Ok(TurnEventPage {
            entries,
            next_cursor,
            truncated,
            rebase_required: None,
        })
    }
}

struct EffectiveMembershipBindingService {
    personal_member: AtomicBool,
    paired: AtomicBool,
    route_kind: ProductConversationRouteKind,
}

impl EffectiveMembershipBindingService {
    fn new(route_kind: ProductConversationRouteKind) -> Self {
        Self {
            personal_member: AtomicBool::new(true),
            paired: AtomicBool::new(true),
            route_kind,
        }
    }

    fn remove_personal_membership(&self) {
        self.personal_member.store(false, Ordering::SeqCst);
    }

    fn unpair(&self) {
        self.paired.store(false, Ordering::SeqCst);
    }

    fn authorize(&self) {
        self.personal_member.store(true, Ordering::SeqCst);
        self.paired.store(true, Ordering::SeqCst);
    }

    fn authorized(&self) -> bool {
        self.personal_member.load(Ordering::SeqCst) && self.paired.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ironclaw_product::ConversationBindingService for EffectiveMembershipBindingService {
    async fn resolve_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ironclaw_product::ResolvedBinding, ironclaw_product::ProductWorkflowError> {
        if !self.authorized() {
            return Err(ironclaw_product::ProductWorkflowError::BindingAccessDenied);
        }
        Ok(binding())
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ironclaw_product::ResolvedBinding, ironclaw_product::ProductWorkflowError> {
        self.resolve_binding(request).await
    }

    async fn resolve_stored_reply_target(
        &self,
        _request: ResolveStoredProductReplyTargetRequest,
    ) -> Result<ResolvedStoredProductReplyTarget, ironclaw_product::ProductWorkflowError> {
        if !self.authorized() {
            return Err(ironclaw_product::ProductWorkflowError::BindingAccessDenied);
        }
        Ok(ResolvedStoredProductReplyTarget {
            adapter_id: ProductAdapterId::new(EXTENSION_ID).expect("adapter"),
            installation_id: AdapterInstallationId::new("install_alpha").expect("installation"),
            external_conversation_ref: ExternalConversationRef::new(
                Some("space-1"),
                "event-conversation",
                None,
                None,
            )
            .expect("conversation"),
            route_kind: self.route_kind,
        })
    }
}

struct EventDeliveryHarness {
    router: Arc<RunDeliveryEventRouter>,
    _handler: Arc<RunDeliveryEventHandler>,
    turns: Arc<EventTurnCoordinator>,
    binding: Arc<EffectiveMembershipBindingService>,
    adapter: Arc<RecordingChannelAdapter>,
    threads: Arc<InMemorySessionThreadService>,
    run_id: TurnRunId,
}

fn event_run_state(run_id: TurnRunId, route_kind: ProductConversationRouteKind) -> TurnRunState {
    let actor = TurnActor::new(user());
    TurnRunState {
        scope: binding_scope(),
        actor: Some(actor.clone()),
        turn_id: TurnId::new(),
        run_id,
        status: TurnStatus::Running,
        accepted_message_ref: AcceptedMessageRef::new("msg:event").expect("ref"),
        source_binding_ref: SourceBindingRef::new("source:event").expect("ref"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply:event").expect("ref"),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        model_usage: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(1),
        product_context: Some(ProductTurnContext::new(
            TurnOriginKind::Inbound,
            Some(match route_kind {
                ProductConversationRouteKind::Direct => TurnSurfaceType::Direct,
                ProductConversationRouteKind::Shared => TurnSurfaceType::Channel,
            }),
            Some(RunOriginAdapter::new(EXTENSION_ID).expect("adapter")),
            TurnOwner::Personal {
                user: actor.user_id,
            },
        )),
        resume_disposition: None,
    }
}

fn build_event_delivery_harness(
    auth_url: Option<&str>,
    route_kind: ProductConversationRouteKind,
) -> EventDeliveryHarness {
    build_event_delivery_harness_with_prompt(
        auth_url.map(|url| {
            Arc::new(StaticAuthPromptSource {
                challenge_kind: AuthPromptChallengeKind::OAuthUrl,
                authorization_url: Some(url.to_string()),
                pairing: None,
                body_override: None,
            }) as Arc<dyn BlockedAuthPromptSource>
        }),
        route_kind,
    )
}

fn build_event_delivery_harness_with_prompt(
    blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    route_kind: ProductConversationRouteKind,
) -> EventDeliveryHarness {
    let run_id = TurnRunId::new();
    let turns = Arc::new(EventTurnCoordinator::new(event_run_state(
        run_id, route_kind,
    )));
    let binding = Arc::new(EffectiveMembershipBindingService::new(route_kind));
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let threads = Arc::new(InMemorySessionThreadService::default());
    let handler = build_event_delivery_handler(
        Arc::clone(&turns),
        Arc::clone(&binding),
        Arc::clone(&adapter),
        Arc::clone(&threads),
        Arc::clone(&store),
        blocked_auth_prompts,
    );
    let router = Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test());
    router.register(EXTENSION_ID, &handler);
    EventDeliveryHarness {
        router,
        _handler: handler,
        turns,
        binding,
        adapter,
        threads,
        run_id,
    }
}

fn build_event_delivery_handler(
    turns: Arc<EventTurnCoordinator>,
    binding: Arc<EffectiveMembershipBindingService>,
    adapter: Arc<RecordingChannelAdapter>,
    threads: Arc<InMemorySessionThreadService>,
    store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
) -> Arc<RunDeliveryEventHandler> {
    build_event_delivery_handler_for_extension(
        turns,
        binding,
        adapter,
        threads,
        store,
        blocked_auth_prompts,
        EXTENSION_ID,
        "install_alpha",
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_event_delivery_handler_for_extension<T>(
    turns: Arc<T>,
    binding: Arc<EffectiveMembershipBindingService>,
    adapter: Arc<RecordingChannelAdapter>,
    threads: Arc<InMemorySessionThreadService>,
    store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    extension_id: &str,
    installation_id: &str,
    current_target_resolver: Option<Arc<dyn CurrentDeliveryTargetResolver>>,
) -> Arc<RunDeliveryEventHandler>
where
    T: TurnCoordinator + 'static,
{
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
        binding_service: Arc::clone(&binding)
            as Arc<dyn ironclaw_product::ConversationBindingService>,
        thread_service: Arc::clone(&threads) as Arc<dyn SessionThreadService>,
        turn_coordinator: Arc::clone(&turns) as Arc<dyn TurnCoordinator>,
        outbound_store: Arc::clone(&store) as Arc<dyn OutboundStateStore>,
        route_store: Arc::clone(&store) as Arc<dyn DeliveredGateRouteStore>,
        communication_preferences: Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>,
        coordinator,
        extension_id: extension_id.to_string(),
        fallback_notice_scope: fallback_scope(),
        approval_context: None,
        blocked_auth_prompts,
        auth_flow_cancel: None,
    };
    let handler = RunDeliveryEventHandler::new(services, extension_id, installation_id);
    Arc::new(match current_target_resolver {
        Some(resolver) => handler.with_current_target_resolver(resolver),
        None => handler,
    })
}

impl EventDeliveryHarness {
    async fn publish(&self, state: TurnRunState, kind: TurnEventKind) {
        let run_id = state.run_id;
        self.router
            .publish(TurnLifecycleEvent::from_run_state(&state, kind, None))
            .await
            .expect("publish lifecycle event");
        self.router.wait_until_run_idle(run_id).await;
    }
}

struct TriggeredLifecycleHarness {
    driver: TriggeredRunDeliveryDriver,
    router: Arc<RunDeliveryEventRouter>,
    turns: Arc<EventTurnCoordinator>,
    adapter: Arc<RecordingChannelAdapter>,
    store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    delivery_store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    threads: Arc<InMemorySessionThreadService>,
    target_resolver: Arc<StaticTriggeredTargetResolver>,
    run_id: TurnRunId,
}

fn build_triggered_lifecycle_harness() -> TriggeredLifecycleHarness {
    let run_id = TurnRunId::new();
    let mut state = event_run_state(run_id, ProductConversationRouteKind::Direct);
    state.product_context = Some(ProductTurnContext::new(
        TurnOriginKind::ScheduledTrigger,
        Some(TurnSurfaceType::Direct),
        Some(RunOriginAdapter::new(EXTENSION_ID).expect("adapter")),
        TurnOwner::Personal { user: user() },
    ));
    let turns = Arc::new(EventTurnCoordinator::new(state));
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let delivery_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let threads = Arc::new(InMemorySessionThreadService::default());
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
        route_store: Arc::clone(&store) as Arc<dyn DeliveredGateRouteStore>,
        communication_preferences: Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>,
        coordinator,
        extension_id: EXTENSION_ID.to_string(),
        fallback_notice_scope: fallback_scope(),
        approval_context: None,
        blocked_auth_prompts: Some(Arc::new(StaticAuthPromptSource {
            challenge_kind: AuthPromptChallengeKind::OAuthUrl,
            authorization_url: Some("https://provider.example/trigger-oauth".to_string()),
            pairing: None,
            body_override: None,
        })),
        auth_flow_cancel: None,
    };
    let target_resolver = Arc::new(StaticTriggeredTargetResolver {
        extension_id: EXTENSION_ID.to_string(),
        conversation: ExternalConversationRef::new(Some("space-1"), "dm-creator", None, None)
            .expect("conversation"),
        personal_dm: true,
        available: AtomicBool::new(true),
    });
    let router = Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test());
    let driver = TriggeredRunDeliveryDriver::with_event_router(
        services,
        Arc::clone(&delivery_store) as Arc<dyn TriggeredRunDeliveryStore>,
        Arc::clone(&target_resolver) as Arc<dyn CurrentDeliveryTargetResolver>,
        agent(),
        Arc::clone(&router),
    );
    TriggeredLifecycleHarness {
        driver,
        router,
        turns,
        adapter,
        store,
        delivery_store,
        threads,
        target_resolver,
        run_id,
    }
}

impl TriggeredLifecycleHarness {
    async fn register(&self) {
        let mut request = triggered_request(self.run_id, false);
        request.delivery_target =
            Some(ReplyTargetBindingRef::new("reply:sealed-source").expect("target"));
        self.driver.on_trigger_submitted(request).await;
    }

    async fn publish(&self, state: TurnRunState, kind: TurnEventKind) {
        let run_id = state.run_id;
        self.router
            .publish(TurnLifecycleEvent::from_run_state(&state, kind, None))
            .await
            .expect("publish triggered lifecycle event");
        self.router.wait_until_run_idle(run_id).await;
    }
}

#[tokio::test]
async fn triggered_delivery_waits_for_lifecycle_event_without_timeout_and_deduplicates() {
    let harness = build_triggered_lifecycle_harness();
    harness.register().await;
    assert!(harness.adapter.texts().is_empty());

    // Longer than the retired focused watcher timeout. No task or permit is
    // waiting; the later durable Completed event owns the delivery.
    tokio::time::sleep(Duration::from_millis(75)).await;
    seed_final_message(&harness.threads, harness.run_id, "scheduled result").await;
    let completed = harness.turns.transition(TurnStatus::Completed, None, 2);
    harness
        .publish(completed.clone(), TurnEventKind::Completed)
        .await;
    harness.publish(completed, TurnEventKind::Completed).await;

    assert_eq!(harness.adapter.texts().len(), 1);
    assert!(harness.adapter.texts()[0].starts_with("scheduled result"));
    assert_eq!(
        wait_for_outcome(&harness.delivery_store, harness.run_id).await,
        TriggeredRunDeliveryOutcomeKind::Delivered
    );
}

#[tokio::test]
async fn triggered_terminal_cleanup_retries_without_resending_the_final_reply() {
    let harness = build_triggered_lifecycle_harness();
    seed_preference(&harness.store).await;
    harness.register().await;

    let blocked = harness.turns.transition(
        TurnStatus::BlockedAuth,
        Some("gate:auth-trigger-cleanup"),
        2,
    );
    tokio::time::timeout(
        Duration::from_secs(2),
        harness.publish(blocked, TurnEventKind::Blocked),
    )
    .await
    .expect("auth prompt delivery completes");
    let blocked_outcome = harness
        .delivery_store
        .load_triggered_run_delivery(harness.run_id)
        .await
        .expect("load blocked outcome");
    assert_eq!(
        harness.adapter.texts().len(),
        1,
        "auth prompt delivered; recorded outcome={blocked_outcome:?}, envelopes={:?}",
        harness.adapter.envelopes()
    );

    seed_final_message(&harness.threads, harness.run_id, "scheduled result").await;
    harness.adapter.reports.lock().expect("reports").extend([
        DeliveryReport {
            parts: vec![PartDeliveryOutcome::Sent {
                vendor_message_ref: Some("final-result-ref".to_string()),
            }],
        },
        DeliveryReport {
            parts: vec![PartDeliveryOutcome::Retryable {
                reason: "provider temporarily unavailable".to_string(),
            }],
        },
        DeliveryReport {
            parts: vec![PartDeliveryOutcome::Retryable {
                reason: "provider still unavailable".to_string(),
            }],
        },
    ]);
    let completed = harness.turns.transition(TurnStatus::Completed, None, 3);
    tokio::time::timeout(
        Duration::from_secs(2),
        harness.publish(completed.clone(), TurnEventKind::Completed),
    )
    .await
    .expect("final delivery and bounded cleanup attempt complete");

    assert_eq!(harness.adapter.texts().len(), 2, "final reply sent once");
    assert_eq!(
        harness.adapter.retracted_refs().len(),
        2,
        "the coordinator exhausted its bounded retry policy"
    );

    tokio::time::timeout(
        Duration::from_secs(2),
        harness.publish(completed.clone(), TurnEventKind::Completed),
    )
    .await
    .expect("terminal replay retries retained cleanup");
    assert_eq!(
        harness.adapter.texts().len(),
        2,
        "cleanup retry must not resend the final reply"
    );
    assert_eq!(
        harness.adapter.retracted_refs().len(),
        3,
        "the retained cleanup responsibility is retried on terminal replay"
    );

    tokio::time::timeout(
        Duration::from_secs(2),
        harness.publish(completed, TurnEventKind::Completed),
    )
    .await
    .expect("settled cleanup releases handler");
    assert_eq!(
        harness.adapter.retracted_refs().len(),
        3,
        "successful cleanup releases the triggered handler"
    );
}

#[tokio::test]
async fn triggered_final_failure_retries_cleanup_without_resending_the_final_reply() {
    let harness = build_triggered_lifecycle_harness();
    seed_preference(&harness.store).await;
    harness.register().await;

    let blocked = harness.turns.transition(
        TurnStatus::BlockedAuth,
        Some("gate:auth-trigger-final-failure"),
        2,
    );
    harness.publish(blocked, TurnEventKind::Blocked).await;
    assert_eq!(harness.adapter.texts().len(), 1, "auth prompt delivered");

    seed_final_message(&harness.threads, harness.run_id, "scheduled result").await;
    harness.adapter.reports.lock().expect("reports").extend([
        DeliveryReport {
            parts: vec![PartDeliveryOutcome::Retryable {
                reason: "final delivery temporarily unavailable".to_string(),
            }],
        },
        DeliveryReport {
            parts: vec![PartDeliveryOutcome::Retryable {
                reason: "final delivery still unavailable".to_string(),
            }],
        },
        DeliveryReport {
            parts: vec![PartDeliveryOutcome::Retryable {
                reason: "cleanup temporarily unavailable".to_string(),
            }],
        },
        DeliveryReport {
            parts: vec![PartDeliveryOutcome::Retryable {
                reason: "cleanup still unavailable".to_string(),
            }],
        },
    ]);
    let completed = harness.turns.transition(TurnStatus::Completed, None, 3);
    harness
        .publish(completed.clone(), TurnEventKind::Completed)
        .await;

    assert_eq!(
        harness.adapter.texts().len(),
        3,
        "the final reply uses only its first bounded delivery attempt"
    );
    assert_eq!(
        harness.adapter.retracted_refs().len(),
        2,
        "terminal final failure still attempts bounded cleanup"
    );
    assert_eq!(
        wait_for_outcome(&harness.delivery_store, harness.run_id).await,
        TriggeredRunDeliveryOutcomeKind::Failed
    );

    harness
        .publish(completed.clone(), TurnEventKind::Completed)
        .await;
    assert_eq!(
        harness.adapter.texts().len(),
        3,
        "cleanup replay must not resend the failed final reply"
    );
    assert_eq!(
        harness.adapter.retracted_refs().len(),
        3,
        "cleanup responsibility remains registered until delivery succeeds"
    );

    harness.publish(completed, TurnEventKind::Completed).await;
    assert_eq!(
        harness.adapter.retracted_refs().len(),
        3,
        "settled cleanup releases the triggered handler"
    );
}

#[tokio::test]
async fn terminal_triggered_event_error_records_failed_and_releases_handler() {
    let harness = build_triggered_lifecycle_harness();
    harness.register().await;
    let completed = harness.turns.transition(TurnStatus::Completed, None, 2);
    harness.turns.fail_next_get_run_state();

    harness
        .publish(completed.clone(), TurnEventKind::Completed)
        .await;
    assert_eq!(
        wait_for_outcome(&harness.delivery_store, harness.run_id).await,
        TriggeredRunDeliveryOutcomeKind::Failed,
        "a one-shot terminal event error must become a durable failed outcome"
    );

    seed_final_message(
        &harness.threads,
        harness.run_id,
        "must not deliver after terminal failure",
    )
    .await;
    harness.publish(completed, TurnEventKind::Completed).await;

    assert!(
        harness.adapter.texts().is_empty(),
        "the failed terminal handler must be released rather than retained for a duplicate event"
    );
}

#[tokio::test]
async fn triggered_delivery_revalidates_current_target_after_registration() {
    let harness = build_triggered_lifecycle_harness();
    harness.register().await;
    harness.target_resolver.revoke();
    seed_final_message(&harness.threads, harness.run_id, "must not leak").await;
    let completed = harness.turns.transition(TurnStatus::Completed, None, 2);
    harness.publish(completed, TurnEventKind::Completed).await;

    assert!(harness.adapter.texts().is_empty());
    assert_eq!(
        wait_for_outcome(&harness.delivery_store, harness.run_id).await,
        TriggeredRunDeliveryOutcomeKind::Denied
    );
}

#[tokio::test]
async fn lifecycle_router_returns_before_egress_and_coalesces_stale_same_run_events() {
    let harness = build_event_delivery_harness(
        Some("https://auth.example/connect"),
        ProductConversationRouteKind::Direct,
    );
    harness.adapter.block_deliveries();

    let submitted = harness.turns.transition(TurnStatus::Running, None, 1);
    tokio::time::timeout(
        Duration::from_millis(25),
        harness.router.publish(TurnLifecycleEvent::from_run_state(
            &submitted,
            TurnEventKind::Submitted,
            None,
        )),
    )
    .await
    .expect("lifecycle publisher must not wait on channel egress")
    .expect("publish lifecycle event");
    harness.adapter.wait_for_started_deliveries(1).await;

    let blocked = harness
        .turns
        .transition(TurnStatus::BlockedAuth, Some("auth:stale"), 2);
    harness
        .router
        .publish(TurnLifecycleEvent::from_run_state(
            &blocked,
            TurnEventKind::Blocked,
            None,
        ))
        .await
        .expect("publish blocked event");

    seed_final_message(&harness.threads, harness.run_id, "serialized final").await;
    let completed = harness.turns.transition(TurnStatus::Completed, None, 3);
    harness
        .router
        .publish(TurnLifecycleEvent::from_run_state(
            &completed,
            TurnEventKind::Completed,
            None,
        ))
        .await
        .expect("publish completed event");
    tokio::task::yield_now().await;
    assert_eq!(
        harness.adapter.started_deliveries.load(Ordering::SeqCst),
        1,
        "blocked/final delivery must remain queued behind this run's progress delivery"
    );

    // The final committed fact replaces the stale queued auth fact. Release
    // the progress notification, final reply, and progress retraction.
    harness.adapter.release_deliveries(3);
    harness.router.wait_until_run_idle(harness.run_id).await;
    assert_eq!(
        harness.adapter.texts(),
        vec![
            "Ironclaw is thinking...".to_string(),
            "serialized final".to_string(),
        ]
    );
    assert_eq!(harness.adapter.retracted_refs().len(), 1);
}

#[tokio::test]
async fn duplicate_lifecycle_events_deliver_once_after_delayed_oauth() {
    let harness = build_event_delivery_harness(
        Some("https://auth.example/connect"),
        ProductConversationRouteKind::Direct,
    );
    let submitted = harness.turns.transition(TurnStatus::Running, None, 1);
    harness
        .publish(submitted.clone(), TurnEventKind::Submitted)
        .await;
    harness.publish(submitted, TurnEventKind::Submitted).await;

    let blocked = harness
        .turns
        .transition(TurnStatus::BlockedAuth, Some("auth:gate:event"), 2);
    harness
        .publish(blocked.clone(), TurnEventKind::Blocked)
        .await;
    harness.publish(blocked, TurnEventKind::Blocked).await;

    let resumed = harness.turns.transition(TurnStatus::Running, None, 3);
    harness
        .publish(resumed.clone(), TurnEventKind::Resumed)
        .await;
    harness.publish(resumed, TurnEventKind::Resumed).await;

    // Longer than the retired observer's focused-test timeout: there is no
    // task or permit waiting here. The later Completed fact independently
    // owns final delivery.
    tokio::time::sleep(Duration::from_millis(75)).await;
    seed_final_message(&harness.threads, harness.run_id, "OAuth completed").await;
    let completed = harness.turns.transition(TurnStatus::Completed, None, 4);
    harness
        .publish(completed.clone(), TurnEventKind::Completed)
        .await;
    harness.publish(completed, TurnEventKind::Completed).await;

    assert_eq!(
        harness.adapter.texts(),
        vec![
            "Ironclaw is thinking...".to_string(),
            "Authentication required\n\nAuthenticate to continue this run.\n\nReply `auth deny auth:gate:event` here to cancel this run.\n\nSetup link: https://auth.example/connect".to_string(),
            "Ironclaw is thinking...".to_string(),
            "OAuth completed".to_string(),
        ]
    );
    assert_eq!(harness.adapter.retracted_refs().len(), 3);
}

#[tokio::test]
async fn post_admission_reconciliation_does_not_duplicate_initial_working_notice() {
    let harness = build_event_delivery_harness(None, ProductConversationRouteKind::Direct);
    let submitted = harness.turns.transition(TurnStatus::Running, None, 1);
    harness.publish(submitted, TurnEventKind::Submitted).await;

    // Admission reconciliation observes a newer durable cursor for the same
    // initial running cycle. It must not produce a second thinking message.
    harness.turns.transition(TurnStatus::Running, None, 2);
    harness
        ._handler
        .reconcile_accepted_user_message(
            harness.router.as_ref(),
            &user_message_envelope(ProductTriggerReason::DirectChat, "evt:initial-reconcile"),
            &accepted_ack(harness.run_id),
        )
        .await
        .expect("post-admission reconciliation");
    harness.router.wait_until_run_idle(harness.run_id).await;

    // A genuine resume is a new working cycle and does get a new indicator.
    let resumed = harness.turns.transition(TurnStatus::Running, None, 3);
    harness.publish(resumed, TurnEventKind::Resumed).await;

    assert_eq!(
        harness
            .adapter
            .texts()
            .iter()
            .filter(|text| text.as_str() == "Ironclaw is thinking...")
            .count(),
        2,
        "one initial working notice plus one resumed-cycle notice"
    );
}

#[tokio::test]
async fn live_delivery_ledger_keeps_only_latest_stage_and_purges_terminal_run() {
    let harness = build_event_delivery_harness(None, ProductConversationRouteKind::Direct);
    let submitted = harness.turns.transition(TurnStatus::Running, None, 1);
    harness.publish(submitted, TurnEventKind::Submitted).await;
    for cursor in 2..=8 {
        let resumed = harness.turns.transition(TurnStatus::Running, None, cursor);
        harness.publish(resumed, TurnEventKind::Resumed).await;
    }

    assert_eq!(
        harness._handler.delivered_claim_count_for_test(),
        1,
        "canonical state makes older delivered stages obsolete"
    );

    let failed = harness.turns.transition(TurnStatus::Failed, None, 9);
    harness.publish(failed, TurnEventKind::Failed).await;
    assert_eq!(
        harness._handler.delivered_claim_count_for_test(),
        0,
        "terminal runs must not remain retained in the in-memory ledger"
    );
}

#[tokio::test]
async fn lifecycle_event_cleanup_retracts_thinking_when_run_fails() {
    let harness = build_event_delivery_harness(None, ProductConversationRouteKind::Direct);
    let submitted = harness.turns.transition(TurnStatus::Running, None, 1);
    harness.publish(submitted, TurnEventKind::Submitted).await;

    let failed = harness.turns.transition(TurnStatus::Failed, None, 2);
    harness.publish(failed, TurnEventKind::Failed).await;

    assert_eq!(
        harness.adapter.texts(),
        vec!["Ironclaw is thinking...".to_string()]
    );
    assert_eq!(harness.adapter.retracted_refs().len(), 1);
}

#[tokio::test]
async fn lifecycle_event_unserviceable_auth_block_cancels_and_keeps_safe_notice() {
    let harness = build_event_delivery_harness(None, ProductConversationRouteKind::Direct);
    let submitted = harness.turns.transition(TurnStatus::Running, None, 1);
    harness.publish(submitted, TurnEventKind::Submitted).await;
    let blocked =
        harness
            .turns
            .transition(TurnStatus::BlockedAuth, Some("auth:gate:credential"), 2);
    harness.publish(blocked, TurnEventKind::Blocked).await;

    let cancelled = harness.turns.transition(TurnStatus::Cancelled, None, 3);
    harness.publish(cancelled, TurnEventKind::Cancelled).await;

    assert_eq!(harness.turns.cancel_call_count(), 1);
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 2);
    assert!(texts[1].contains("Ironclaw web app"), "{}", texts[1]);
    assert_eq!(
        harness.adapter.retracted_refs().len(),
        1,
        "cancellation retracts the transient working message, not the terminal setup notice"
    );
}

#[tokio::test]
async fn lifecycle_event_non_serviceable_typed_auth_cancels_with_exact_safe_notice() {
    const GENERIC_NOTICE: &str = "This authentication step can't be completed in chat. Open the Ironclaw web app to review it, then ask me again here.";
    const MANUAL_TOKEN_NOTICE: &str = "Setting this up needs a credential (an API key or token). Sharing one here is a security risk — anything entered in chat is stored in the conversation — so credential-based connections can only be set up in the Ironclaw web app. Connect it there, then ask me again here.";
    const PRIVATE_PROMPT_MATERIAL: &str = "private-prompt-material-must-not-be-echoed";

    for (challenge_kind, expected_notice) in [
        (AuthPromptChallengeKind::ManualToken, MANUAL_TOKEN_NOTICE),
        (AuthPromptChallengeKind::Other, GENERIC_NOTICE),
        (AuthPromptChallengeKind::Pairing, GENERIC_NOTICE),
        (AuthPromptChallengeKind::OAuthUrl, GENERIC_NOTICE),
    ] {
        let harness = build_event_delivery_harness_with_prompt(
            Some(Arc::new(StaticAuthPromptSource {
                challenge_kind,
                authorization_url: None,
                pairing: None,
                body_override: Some(PRIVATE_PROMPT_MATERIAL.to_string()),
            })),
            ProductConversationRouteKind::Direct,
        );
        let blocked =
            harness
                .turns
                .transition(TurnStatus::BlockedAuth, Some("auth:gate:private"), 2);
        harness.publish(blocked, TurnEventKind::Blocked).await;

        assert_eq!(
            harness.turns.cancel_call_count(),
            1,
            "{challenge_kind:?} must not leave the run blocked"
        );
        assert_eq!(
            harness.adapter.texts(),
            vec![expected_notice.to_string()],
            "{challenge_kind:?} must produce only its terminal-safe WebUI notice"
        );
        assert!(
            !harness.adapter.texts()[0].contains(PRIVATE_PROMPT_MATERIAL),
            "{challenge_kind:?} must not echo prompt or credential material"
        );
    }
}

#[tokio::test]
async fn lifecycle_event_pairing_without_url_stays_blocked_with_actionable_guidance() {
    let harness = build_event_delivery_harness_with_prompt(
        Some(Arc::new(StaticAuthPromptSource {
            challenge_kind: AuthPromptChallengeKind::Pairing,
            authorization_url: None,
            pairing: Some(PairingPromptView {
                channel: "fixture".to_string(),
                display_name: "Fixture Chat".to_string(),
                instructions: "Open the link or send `/start <code>` to the configured bot."
                    .to_string(),
                code: "PAIR42AB".to_string(),
                deep_link: Some("https://example.test/start=PAIR42AB".to_string()),
                expires_at: chrono::DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
                    .expect("expiry")
                    .with_timezone(&Utc),
            }),
            body_override: None,
        })),
        ProductConversationRouteKind::Direct,
    );
    let blocked = harness
        .turns
        .transition(TurnStatus::BlockedAuth, Some("auth:gate:pairing"), 2);
    harness.publish(blocked, TurnEventKind::Blocked).await;

    assert_eq!(
        harness.turns.cancel_call_count(),
        0,
        "typed pairing is serviceable without an OAuth URL"
    );
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1);
    assert!(
        texts[0].contains("Open the link or send `/start <code>`"),
        "pairing guidance must be actionable: {}",
        texts[0]
    );
    assert!(
        texts[0].contains("PAIR42AB"),
        "missing pairing code: {}",
        texts[0]
    );
    assert!(
        texts[0].contains("https://example.test/start=PAIR42AB"),
        "missing pairing deep link: {}",
        texts[0]
    );
    assert!(
        texts[0].contains("2030-01-01T00:00:00+00:00"),
        "missing pairing expiry: {}",
        texts[0]
    );
    assert!(
        !texts[0].contains("/pair"),
        "generic delivery must not invent transport commands: {}",
        texts[0]
    );
}

#[tokio::test]
async fn lifecycle_event_shared_auth_prompt_never_contains_oauth_setup_url() {
    let harness = build_event_delivery_harness(
        Some("https://auth.example/connect"),
        ProductConversationRouteKind::Shared,
    );
    let blocked = harness
        .turns
        .transition(TurnStatus::BlockedAuth, Some("auth:gate:shared"), 2);
    harness.publish(blocked, TurnEventKind::Blocked).await;

    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1);
    assert!(texts[0].contains("Authentication required"), "{}", texts[0]);
    assert!(!texts[0].contains("Setup link:"), "{}", texts[0]);
}

#[tokio::test]
async fn channel_removal_or_unpairing_revokes_delayed_delivery() {
    let harness = build_event_delivery_harness(None, ProductConversationRouteKind::Direct);
    harness.binding.remove_personal_membership();
    seed_final_message(&harness.threads, harness.run_id, "must not leak").await;
    let completed = harness.turns.transition(TurnStatus::Completed, None, 2);
    harness.publish(completed, TurnEventKind::Completed).await;
    assert!(harness.adapter.texts().is_empty());

    let second = build_event_delivery_harness(None, ProductConversationRouteKind::Direct);
    second.binding.unpair();
    seed_final_message(&second.threads, second.run_id, "still must not leak").await;
    let completed = second.turns.transition(TurnStatus::Completed, None, 2);
    second.publish(completed, TurnEventKind::Completed).await;
    assert!(second.adapter.texts().is_empty());
}

#[tokio::test]
async fn durable_handoff_drain_reaches_later_page_when_first_page_is_deferred() {
    const HANDOFF_COUNT: u64 = 257;

    let mut states = HashMap::new();
    let mut events = Vec::new();
    let mut deliverable_run_id = None;
    for cursor in 1..=HANDOFF_COUNT {
        let run_id = TurnRunId::new();
        let mut state = event_run_state(run_id, ProductConversationRouteKind::Direct);
        state.status = TurnStatus::Completed;
        state.event_cursor = EventCursor(cursor);
        // Every filler stays a channel-origin (Inbound) run with no finalized
        // message: it materializes a handoff (Inbound always does) but defers at
        // drain, while only the last page's run has a reply to deliver. A
        // context-less run is now skipped at materialization, so it can no
        // longer stand in as a durable-but-deferred filler.
        if cursor == HANDOFF_COUNT {
            deliverable_run_id = Some(run_id);
        }
        events.push(TurnLifecycleEvent::from_run_state(
            &state,
            TurnEventKind::Completed,
            None,
        ));
        states.insert(run_id, state);
    }
    let deliverable_run_id = deliverable_run_id.expect("deliverable run");
    let turns = Arc::new(RunMapTurnCoordinator { states });
    let binding = Arc::new(EffectiveMembershipBindingService::new(
        ProductConversationRouteKind::Direct,
    ));
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let threads = Arc::new(InMemorySessionThreadService::default());
    seed_final_message(&threads, deliverable_run_id, "later page reply").await;
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let handler = build_event_delivery_handler_for_extension(
        Arc::clone(&turns),
        binding,
        Arc::clone(&adapter),
        threads,
        Arc::clone(&store),
        None,
        EXTENSION_ID,
        "install_alpha",
        None,
    );
    let event_log = Arc::new(StaticDurableTurnEventLog::new(events));
    let router = RunDeliveryEventRouter::new(
        event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(turns as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    router.register(EXTENSION_ID, &handler);
    router.wait_until_durable_replay_idle().await;

    assert_eq!(
        adapter.texts(),
        vec!["later page reply"],
        "a full first page without progress must not starve a later deliverable handoff"
    );
}

#[tokio::test]
async fn cross_channel_handoff_stays_pending_when_source_cleanup_fails_then_reopen_converges_without_duplicate_destination_send()
 {
    const DESTINATION_EXTENSION_ID: &str = "a-destination";

    let run_id = TurnRunId::new();
    let turns = Arc::new(EventTurnCoordinator::new(event_run_state(
        run_id,
        ProductConversationRouteKind::Direct,
    )));
    let source_turns = Arc::new(FailNextGetRunStateCoordinator::new(Arc::clone(&turns)));
    let binding = Arc::new(EffectiveMembershipBindingService::new(
        ProductConversationRouteKind::Direct,
    ));
    let source_adapter = Arc::new(RecordingChannelAdapter::new());
    let destination_adapter = Arc::new(RecordingChannelAdapter::new());
    let threads = Arc::new(InMemorySessionThreadService::default());
    let filesystem = ironclaw_outbound::test_support::in_memory_backed_outbound_filesystem();
    #[allow(clippy::disallowed_methods)]
    let store = Arc::new(FilesystemOutboundStateStore::new(Arc::clone(&filesystem)));
    let source_handler = build_event_delivery_handler_for_extension(
        Arc::clone(&source_turns),
        Arc::clone(&binding),
        Arc::clone(&source_adapter),
        Arc::clone(&threads),
        Arc::clone(&store),
        None,
        EXTENSION_ID,
        "install_alpha",
        None,
    );

    let source_router = RunDeliveryEventRouter::new_ephemeral_for_test();
    source_router.register(EXTENSION_ID, &source_handler);
    let running = turns.transition(TurnStatus::Running, None, 1);
    source_router
        .publish(TurnLifecycleEvent::from_run_state(
            &running,
            TurnEventKind::Submitted,
            None,
        ))
        .await
        .expect("publish source progress event");
    source_router.wait_until_run_idle(run_id).await;
    assert_eq!(
        source_adapter.texts(),
        vec!["Ironclaw is thinking..."],
        "the source owner has a placeholder that completion must retract"
    );

    let target_ref =
        ReplyTargetBindingRef::new("reply:test:cross-channel").expect("destination target");
    let completed = turns.transition(TurnStatus::Completed, None, 2);
    store
        .put_run_final_reply_target(RunFinalReplyTargetRecord {
            run_id,
            scope: completed.scope.clone(),
            actor: completed.actor.clone().expect("completed run actor"),
            destination: RunFinalReplyDestination::External {
                reply_target_binding_ref: target_ref,
            },
        })
        .await
        .expect("persist explicit destination");
    seed_final_message(&threads, run_id, "routed reply").await;

    let destination_resolver = Arc::new(StaticTriggeredTargetResolver {
        extension_id: DESTINATION_EXTENSION_ID.to_string(),
        conversation: ExternalConversationRef::new(
            Some("destination-space"),
            "destination-conversation",
            None,
            None,
        )
        .expect("destination conversation"),
        personal_dm: true,
        available: AtomicBool::new(true),
    });
    let destination_handler = build_event_delivery_handler_for_extension(
        Arc::clone(&turns),
        Arc::clone(&binding),
        Arc::clone(&destination_adapter),
        Arc::clone(&threads),
        Arc::clone(&store),
        None,
        DESTINATION_EXTENSION_ID,
        "install_destination",
        Some(Arc::clone(&destination_resolver) as Arc<dyn CurrentDeliveryTargetResolver>),
    );
    let event_log = Arc::new(StaticDurableTurnEventLog::new(vec![
        TurnLifecycleEvent::from_run_state(&completed, TurnEventKind::Completed, None),
    ]));
    let replay_router = RunDeliveryEventRouter::new(
        Arc::clone(&event_log) as Arc<dyn TurnEventProjectionSource>,
        run_state_store(Arc::clone(&turns) as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    // Destination registration can race source graph startup. A destination
    // send alone cannot settle the handoff while source cleanup is absent.
    replay_router.register(DESTINATION_EXTENSION_ID, &destination_handler);
    replay_router.wait_until_durable_replay_idle().await;

    assert_eq!(destination_adapter.texts(), vec!["routed reply"]);
    assert!(
        source_adapter.retracted_refs().is_empty(),
        "the source owner is not registered yet"
    );
    assert_eq!(
        store
            .list_pending_run_final_reply_handoffs(16)
            .await
            .expect("pending handoffs")
            .len(),
        1,
        "destination delivery must not discard an absent source owner's cleanup"
    );

    // Registry iteration order is intentionally unspecified. Every live owner
    // must be visited and the handoff cannot settle while any owner defers.
    source_turns.fail_next_get_run_state();
    replay_router.register(EXTENSION_ID, &source_handler);
    replay_router.wait_until_durable_replay_idle().await;

    assert_eq!(
        destination_adapter.texts(),
        vec!["routed reply"],
        "source registration must not duplicate the settled destination send"
    );
    assert!(
        source_adapter.retracted_refs().is_empty(),
        "the transiently unavailable source owner has not cleaned up yet"
    );
    assert_eq!(
        store
            .list_pending_run_final_reply_handoffs(16)
            .await
            .expect("pending handoffs")
            .len(),
        1,
        "one settled owner must not discard another owner's retryable cleanup"
    );

    #[allow(clippy::disallowed_methods)]
    let reopened_store = Arc::new(FilesystemOutboundStateStore::new(Arc::clone(&filesystem)));
    let reopened_turns = Arc::new(EventTurnCoordinator::new(completed.clone()));
    let reopened_source_handler = build_event_delivery_handler(
        Arc::clone(&reopened_turns),
        Arc::clone(&binding),
        Arc::clone(&source_adapter),
        Arc::clone(&threads),
        Arc::clone(&reopened_store),
        None,
    );
    let reopened_destination_handler = build_event_delivery_handler_for_extension(
        Arc::clone(&reopened_turns),
        Arc::clone(&binding),
        Arc::clone(&destination_adapter),
        Arc::clone(&threads),
        Arc::clone(&reopened_store),
        None,
        DESTINATION_EXTENSION_ID,
        "install_destination",
        Some(Arc::clone(&destination_resolver) as Arc<dyn CurrentDeliveryTargetResolver>),
    );
    let reopened_router = RunDeliveryEventRouter::new(
        Arc::clone(&event_log) as Arc<dyn TurnEventProjectionSource>,
        run_state_store(Arc::clone(&reopened_turns) as Arc<dyn TurnCoordinator>),
        Arc::clone(&reopened_store) as Arc<dyn OutboundStateStore>,
    );
    source_adapter.reports.lock().expect("reports").extend([
        DeliveryReport {
            parts: vec![PartDeliveryOutcome::Retryable {
                reason: "cleanup temporarily unavailable".to_string(),
            }],
        },
        DeliveryReport {
            parts: vec![PartDeliveryOutcome::Retryable {
                reason: "cleanup still unavailable".to_string(),
            }],
        },
    ]);
    reopened_router.register(DESTINATION_EXTENSION_ID, &reopened_destination_handler);
    reopened_router.wait_until_durable_replay_idle().await;
    reopened_router.register(EXTENSION_ID, &reopened_source_handler);
    reopened_router.wait_until_durable_replay_idle().await;

    assert_eq!(
        destination_adapter.texts(),
        vec!["routed reply"],
        "the deterministic destination attempt must suppress a duplicate send on reopen"
    );
    assert_eq!(
        source_adapter.retracted_refs().len(),
        2,
        "the coordinator exhausts one bounded cleanup attempt"
    );
    assert_eq!(
        source_adapter.texts(),
        vec!["Ironclaw is thinking..."],
        "the explicit result must not be duplicated to the source"
    );
    assert_eq!(
        reopened_store
            .list_pending_run_final_reply_handoffs(16)
            .await
            .expect("pending handoffs")
            .len(),
        1,
        "a failed provider cleanup must keep the cross-channel handoff pending"
    );
    let cleanup_request = RunDeliveryCleanupRequest {
        scope: completed.scope.clone(),
        run_id,
        adapter: RunOriginAdapter::new(EXTENSION_ID).expect("source adapter"),
    };
    assert_eq!(
        reopened_store
            .load_run_delivery_cleanup(cleanup_request.clone())
            .await
            .expect("durable cleanup records")
            .len(),
        1,
        "a failed provider cleanup must retain its durable message reference"
    );
    let first_cleanup_attempt_ids = source_adapter
        .envelopes()
        .into_iter()
        .filter(|envelope| {
            envelope
                .parts
                .iter()
                .any(|part| matches!(part, OutboundPart::Retract { .. }))
        })
        .map(|envelope| envelope.delivery_attempt_id)
        .collect::<Vec<_>>();
    assert_eq!(first_cleanup_attempt_ids.len(), 2);
    assert_eq!(
        first_cleanup_attempt_ids[0], first_cleanup_attempt_ids[1],
        "bounded provider retries belong to one durable delivery attempt"
    );

    #[allow(clippy::disallowed_methods)]
    let settled_store = Arc::new(FilesystemOutboundStateStore::new(filesystem));
    let settled_turns = Arc::new(EventTurnCoordinator::new(completed.clone()));
    let settled_source_handler = build_event_delivery_handler(
        Arc::clone(&settled_turns),
        Arc::clone(&binding),
        Arc::clone(&source_adapter),
        Arc::clone(&threads),
        Arc::clone(&settled_store),
        None,
    );
    let settled_destination_handler = build_event_delivery_handler_for_extension(
        Arc::clone(&settled_turns),
        Arc::clone(&binding),
        Arc::clone(&destination_adapter),
        Arc::clone(&threads),
        Arc::clone(&settled_store),
        None,
        DESTINATION_EXTENSION_ID,
        "install_destination",
        Some(destination_resolver as Arc<dyn CurrentDeliveryTargetResolver>),
    );
    let settled_router = RunDeliveryEventRouter::new(
        event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(Arc::clone(&settled_turns) as Arc<dyn TurnCoordinator>),
        Arc::clone(&settled_store) as Arc<dyn OutboundStateStore>,
    );
    settled_router.register(DESTINATION_EXTENSION_ID, &settled_destination_handler);
    settled_router.wait_until_durable_replay_idle().await;
    settled_router.register(EXTENSION_ID, &settled_source_handler);
    settled_router.wait_until_durable_replay_idle().await;

    assert_eq!(
        destination_adapter.texts(),
        vec!["routed reply"],
        "cleanup retry must not duplicate the settled destination send"
    );
    assert_eq!(
        source_adapter.retracted_refs().len(),
        3,
        "the recovered source owner retries and settles its cleanup"
    );
    let cleanup_attempt_ids = source_adapter
        .envelopes()
        .into_iter()
        .filter(|envelope| {
            envelope
                .parts
                .iter()
                .any(|part| matches!(part, OutboundPart::Retract { .. }))
        })
        .map(|envelope| envelope.delivery_attempt_id)
        .collect::<Vec<_>>();
    assert_eq!(cleanup_attempt_ids.len(), 3);
    assert_ne!(
        cleanup_attempt_ids[1], cleanup_attempt_ids[2],
        "a later intentional cleanup retry gets a fresh delivery attempt id"
    );
    assert!(
        settled_store
            .load_run_delivery_cleanup(cleanup_request)
            .await
            .expect("settled cleanup records")
            .is_empty(),
        "successful retry completes the durable cleanup record"
    );
    assert!(
        settled_store
            .list_pending_run_final_reply_handoffs(16)
            .await
            .expect("pending handoffs")
            .is_empty(),
        "the handoff settles after every required owner converges"
    );
}

#[tokio::test]
async fn completed_reply_replays_after_crash_before_volatile_observer_and_reopen() {
    let run_id = TurnRunId::new();
    let turns = Arc::new(EventTurnCoordinator::new(event_run_state(
        run_id,
        ProductConversationRouteKind::Direct,
    )));
    let completed_state = turns.transition(TurnStatus::Completed, None, 2);
    let completed_event =
        TurnLifecycleEvent::from_run_state(&completed_state, TurnEventKind::Completed, None);
    let event_log = Arc::new(StaticDurableTurnEventLog::new(vec![completed_event]));
    let binding = Arc::new(EffectiveMembershipBindingService::new(
        ProductConversationRouteKind::Direct,
    ));
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let threads = Arc::new(InMemorySessionThreadService::default());
    seed_final_message(&threads, run_id, "reply recovered after restart").await;

    let filesystem = ironclaw_outbound::test_support::in_memory_backed_outbound_filesystem();
    #[allow(clippy::disallowed_methods)]
    let first_store = Arc::new(FilesystemOutboundStateStore::new(Arc::clone(&filesystem)));
    let first_handler = build_event_delivery_handler(
        Arc::clone(&turns),
        Arc::clone(&binding),
        Arc::clone(&adapter),
        Arc::clone(&threads),
        Arc::clone(&first_store),
        None,
    );

    // The event is already committed, and no volatile publish reaches this
    // process. Startup catch-up alone must recover it.
    let first_router = RunDeliveryEventRouter::new(
        Arc::clone(&event_log) as Arc<dyn TurnEventProjectionSource>,
        run_state_store(Arc::clone(&turns) as Arc<dyn TurnCoordinator>),
        Arc::clone(&first_store) as Arc<dyn OutboundStateStore>,
    );
    first_router.register(EXTENSION_ID, &first_handler);
    first_router.wait_until_durable_replay_idle().await;

    assert_eq!(adapter.texts(), vec!["reply recovered after restart"]);
    assert_eq!(
        adapter.envelopes()[0].target.conversation.conversation_id(),
        "event-conversation",
        "replay must reopen the exact sealed source route"
    );
    assert!(
        first_store
            .list_pending_run_final_reply_handoffs(16)
            .await
            .expect("pending handoffs")
            .is_empty(),
        "handoff is deleted only after the coordinator reaches a terminal outcome"
    );
    assert_eq!(
        first_store
            .load_run_final_reply_handoff_cursor()
            .await
            .expect("handoff cursor"),
        EventCursor(2)
    );

    // Reopen the durable store and replay the same authoritative log. The
    // cursor and deterministic outbound attempt prevent a duplicate send.
    #[allow(clippy::disallowed_methods)]
    let reopened_store = Arc::new(FilesystemOutboundStateStore::new(filesystem));
    let reopened_handler = build_event_delivery_handler(
        Arc::clone(&turns),
        binding,
        Arc::clone(&adapter),
        threads,
        Arc::clone(&reopened_store),
        None,
    );
    let reopened_router = RunDeliveryEventRouter::new(
        event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(Arc::clone(&turns) as Arc<dyn TurnCoordinator>),
        reopened_store as Arc<dyn OutboundStateStore>,
    );
    reopened_router.register(EXTENSION_ID, &reopened_handler);
    reopened_router.wait_until_durable_replay_idle().await;
    assert_eq!(adapter.texts().len(), 1, "restart replay is idempotent");
}

#[tokio::test]
async fn completed_reply_replay_is_duplicate_safe_across_workers() {
    let run_id = TurnRunId::new();
    let turns = Arc::new(EventTurnCoordinator::new(event_run_state(
        run_id,
        ProductConversationRouteKind::Direct,
    )));
    let completed = turns.transition(TurnStatus::Completed, None, 2);
    let event_log = Arc::new(StaticDurableTurnEventLog::new(vec![
        TurnLifecycleEvent::from_run_state(&completed, TurnEventKind::Completed, None),
    ]));
    let binding = Arc::new(EffectiveMembershipBindingService::new(
        ProductConversationRouteKind::Direct,
    ));
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let threads = Arc::new(InMemorySessionThreadService::default());
    seed_final_message(&threads, run_id, "one provider send").await;
    let filesystem = ironclaw_outbound::test_support::in_memory_backed_outbound_filesystem();
    #[allow(clippy::disallowed_methods)]
    let store_a = Arc::new(FilesystemOutboundStateStore::new(Arc::clone(&filesystem)));
    #[allow(clippy::disallowed_methods)]
    let store_b = Arc::new(FilesystemOutboundStateStore::new(filesystem));
    let handler_a = build_event_delivery_handler(
        Arc::clone(&turns),
        Arc::clone(&binding),
        Arc::clone(&adapter),
        Arc::clone(&threads),
        Arc::clone(&store_a),
        None,
    );
    let handler_b = build_event_delivery_handler(
        Arc::clone(&turns),
        binding,
        Arc::clone(&adapter),
        threads,
        Arc::clone(&store_b),
        None,
    );
    let router_a = RunDeliveryEventRouter::new(
        Arc::clone(&event_log) as Arc<dyn TurnEventProjectionSource>,
        run_state_store(Arc::clone(&turns) as Arc<dyn TurnCoordinator>),
        store_a as Arc<dyn OutboundStateStore>,
    );
    let router_b = RunDeliveryEventRouter::new(
        event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(turns as Arc<dyn TurnCoordinator>),
        store_b as Arc<dyn OutboundStateStore>,
    );
    router_a.register(EXTENSION_ID, &handler_a);
    router_b.register(EXTENSION_ID, &handler_b);
    tokio::join!(
        router_a.wait_until_durable_replay_idle(),
        router_b.wait_until_durable_replay_idle()
    );
    assert_eq!(
        adapter.texts(),
        vec!["one provider send"],
        "the durable coordinator send claim must admit only one worker"
    );
}

#[tokio::test]
async fn permanent_channel_failure_is_terminal_across_later_recovery() {
    let run_id = TurnRunId::new();
    let turns = Arc::new(EventTurnCoordinator::new(event_run_state(
        run_id,
        ProductConversationRouteKind::Direct,
    )));
    let completed = turns.transition(TurnStatus::Completed, None, 2);
    let completed_event =
        TurnLifecycleEvent::from_run_state(&completed, TurnEventKind::Completed, None);
    let event_log = Arc::new(StaticDurableTurnEventLog::new(vec![
        completed_event.clone(),
    ]));
    let binding = Arc::new(EffectiveMembershipBindingService::new(
        ProductConversationRouteKind::Direct,
    ));
    let adapter = Arc::new(RecordingChannelAdapter::new());
    adapter
        .reports
        .lock()
        .expect("reports")
        .push_back(DeliveryReport {
            parts: vec![PartDeliveryOutcome::Permanent {
                reason: "provider rejected permanently".to_string(),
            }],
        });
    let threads = Arc::new(InMemorySessionThreadService::default());
    seed_final_message(&threads, run_id, "terminal provider failure").await;
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let handler = build_event_delivery_handler(
        Arc::clone(&turns),
        Arc::clone(&binding),
        Arc::clone(&adapter),
        Arc::clone(&threads),
        Arc::clone(&store),
        None,
    );
    let router = RunDeliveryEventRouter::new(
        Arc::clone(&event_log) as Arc<dyn TurnEventProjectionSource>,
        run_state_store(Arc::clone(&turns) as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    router.register(EXTENSION_ID, &handler);
    router.wait_until_durable_replay_idle().await;
    assert_eq!(
        adapter.envelopes().len(),
        1,
        "a permanent provider result is not retried by a parallel watcher"
    );

    // Simulate a recovery process encountering the same rebuildable handoff
    // again. The deterministic coordinator attempt is already terminal, so
    // it suppresses a second provider call and settles the handoff.
    store
        .put_run_final_reply_handoff(ironclaw_outbound::RunFinalReplyHandoffRecord {
            event_cursor: completed_event.cursor,
            scope: completed_event.scope.clone(),
            run_id,
        })
        .await
        .expect("reinsert handoff for recovery");
    let recovery_handler = build_event_delivery_handler(
        Arc::clone(&turns),
        binding,
        Arc::clone(&adapter),
        threads,
        Arc::clone(&store),
        None,
    );
    let recovery_router = RunDeliveryEventRouter::new(
        event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(turns as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    recovery_router.register(EXTENSION_ID, &recovery_handler);
    recovery_router.wait_until_durable_replay_idle().await;
    assert_eq!(adapter.envelopes().len(), 1);
    assert!(
        store
            .list_pending_run_final_reply_handoffs(16)
            .await
            .expect("pending handoffs")
            .is_empty()
    );

    // Terminal evidence is scoped to this exact run/projection. A later,
    // independent run using the same channel must still reach the provider;
    // the old permanent result must neither retry nor poison future sends.
    let later_run_id = TurnRunId::new();
    let later_turns = Arc::new(EventTurnCoordinator::new(event_run_state(
        later_run_id,
        ProductConversationRouteKind::Direct,
    )));
    let later_completed = later_turns.transition(TurnStatus::Completed, None, 3);
    let later_event_log = Arc::new(StaticDurableTurnEventLog::new(vec![
        TurnLifecycleEvent::from_run_state(&later_completed, TurnEventKind::Completed, None),
    ]));
    let later_binding = Arc::new(EffectiveMembershipBindingService::new(
        ProductConversationRouteKind::Direct,
    ));
    let later_threads = Arc::new(InMemorySessionThreadService::default());
    seed_final_message(&later_threads, later_run_id, "later channel reply").await;
    let later_handler = build_event_delivery_handler(
        Arc::clone(&later_turns),
        later_binding,
        Arc::clone(&adapter),
        later_threads,
        Arc::clone(&store),
        None,
    );
    let later_router = RunDeliveryEventRouter::new(
        later_event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(later_turns as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    later_router.register(EXTENSION_ID, &later_handler);
    later_router.wait_until_durable_replay_idle().await;

    assert_eq!(
        adapter.envelopes().len(),
        2,
        "the later independent run must make one fresh provider call"
    );
    let attempts = store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("delivery attempts");
    let original_attempt = attempts
        .iter()
        .find(|attempt| attempt.candidate.turn_run_id == Some(run_id))
        .expect("original run attempt");
    assert_eq!(
        original_attempt.status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed,
        "the original permanent result remains terminal and nonduplicated"
    );
    let later_attempt = attempts
        .iter()
        .find(|attempt| attempt.candidate.turn_run_id == Some(later_run_id))
        .expect("later run attempt");
    assert_eq!(
        later_attempt.status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered,
        "old terminal evidence must not poison a later run"
    );
}

#[tokio::test]
async fn completed_reply_replay_fails_loud_without_skipping_a_retention_gap() {
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let event_log = Arc::new(StaticDurableTurnEventLog::requiring_rebase(EventCursor(9)));
    // The retention-gap short-circuits materialization before any run-state
    // lookup, so the run-state seam is never exercised here.
    let router = RunDeliveryEventRouter::new(
        event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(Arc::new(EventTurnCoordinator::new(event_run_state(
            TurnRunId::new(),
            ProductConversationRouteKind::Direct,
        ))) as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    router.wait_until_durable_replay_idle().await;
    assert_eq!(
        store
            .load_run_final_reply_handoff_cursor()
            .await
            .expect("handoff cursor"),
        EventCursor::default(),
        "retention gaps must never advance the consumer cursor"
    );
}

#[tokio::test]
async fn completed_reply_replay_settles_without_send_after_membership_removal() {
    let run_id = TurnRunId::new();
    let turns = Arc::new(EventTurnCoordinator::new(event_run_state(
        run_id,
        ProductConversationRouteKind::Direct,
    )));
    let completed = turns.transition(TurnStatus::Completed, None, 2);
    let event_log = Arc::new(StaticDurableTurnEventLog::new(vec![
        TurnLifecycleEvent::from_run_state(&completed, TurnEventKind::Completed, None),
    ]));
    let binding = Arc::new(EffectiveMembershipBindingService::new(
        ProductConversationRouteKind::Direct,
    ));
    binding.remove_personal_membership();
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let threads = Arc::new(InMemorySessionThreadService::default());
    seed_final_message(&threads, run_id, "must not leak").await;
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let handler = build_event_delivery_handler(
        Arc::clone(&turns),
        binding,
        Arc::clone(&adapter),
        threads,
        Arc::clone(&store),
        None,
    );
    let router = RunDeliveryEventRouter::new(
        event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(turns as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    router.register(EXTENSION_ID, &handler);
    router.wait_until_durable_replay_idle().await;
    assert!(adapter.texts().is_empty());
    let attempts = store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("delivery attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed,
        "current membership denial must be recorded as a durable terminal outcome"
    );
    assert_eq!(
        attempts[0].failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::AuthorizationRevoked),
        "the durable terminal failure must retain the authorization-denial reason"
    );
    assert!(
        store
            .list_pending_run_final_reply_handoffs(16)
            .await
            .expect("pending handoffs")
            .is_empty(),
        "current authority denial is a terminal fail-closed handoff outcome"
    );
}

/// Drive a completed run with no external channel destination through the
/// durable replay and assert it never materializes a handoff (which no channel
/// handler would ever settle) yet still advances the consumer cursor past the
/// skipped event, so later drains cannot re-scan it.
async fn assert_completed_run_materializes_no_channel_handoff(state: TurnRunState) {
    let run_id = state.run_id;
    let turns = Arc::new(EventTurnCoordinator::new(state.clone()));
    let event_log = Arc::new(StaticDurableTurnEventLog::new(vec![
        TurnLifecycleEvent::from_run_state(&state, TurnEventKind::Completed, None),
    ]));
    let binding = Arc::new(EffectiveMembershipBindingService::new(
        ProductConversationRouteKind::Direct,
    ));
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let threads = Arc::new(InMemorySessionThreadService::default());
    // A finalized reply exists: the point is that it must not be fanned out to a
    // channel, not that there is nothing to say.
    seed_final_message(&threads, run_id, "must not leak to a channel").await;
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let handler = build_event_delivery_handler(
        Arc::clone(&turns),
        binding,
        Arc::clone(&adapter),
        threads,
        Arc::clone(&store),
        None,
    );
    let router = RunDeliveryEventRouter::new(
        event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(turns as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    router.register(EXTENSION_ID, &handler);
    router.wait_until_durable_replay_idle().await;

    assert!(
        adapter.texts().is_empty(),
        "a completed run with no external channel destination must not deliver to a channel"
    );
    assert!(
        store
            .list_pending_run_final_reply_handoffs(16)
            .await
            .expect("pending handoffs")
            .is_empty(),
        "a completed run no channel handler can own must not leak a durable pending handoff"
    );
    assert_eq!(
        store
            .load_run_final_reply_handoff_cursor()
            .await
            .expect("handoff cursor"),
        EventCursor(2),
        "the consumer cursor advances past the skipped event so later drains never re-scan it"
    );
}

#[tokio::test]
async fn completed_webui_webapp_run_does_not_leak_a_durable_handoff() {
    let run_id = TurnRunId::new();
    let mut state = event_run_state(run_id, ProductConversationRouteKind::Direct);
    state.status = TurnStatus::Completed;
    state.event_cursor = EventCursor(2);
    let actor = state.actor.clone().expect("webui run actor");
    // A pure WebUI chat: no channel adapter, and no sealed external target, so
    // the answer lives in the web app. Every normal chat completion looks like
    // this; it must not accrete a permanent pending handoff.
    state.product_context = Some(ProductTurnContext::new(
        TurnOriginKind::WebUi,
        Some(TurnSurfaceType::Direct),
        None,
        TurnOwner::Personal {
            user: actor.user_id,
        },
    ));
    assert_completed_run_materializes_no_channel_handoff(state).await;
}

#[tokio::test]
async fn completed_scheduled_trigger_run_does_not_leak_a_durable_handoff() {
    let run_id = TurnRunId::new();
    let mut state = event_run_state(run_id, ProductConversationRouteKind::Direct);
    state.status = TurnStatus::Completed;
    state.event_cursor = EventCursor(2);
    // A scheduled trigger is delivered by the separate volatile triggered
    // driver, never by this durable channel path, so a handoff here is pure
    // leak even though the run carries a channel adapter.
    state.product_context = Some(ProductTurnContext::new(
        TurnOriginKind::ScheduledTrigger,
        Some(TurnSurfaceType::Direct),
        Some(RunOriginAdapter::new(EXTENSION_ID).expect("adapter")),
        TurnOwner::Personal { user: user() },
    ));
    assert_completed_run_materializes_no_channel_handoff(state).await;
}

#[tokio::test]
async fn completed_final_delivery_dedups_across_a_post_completed_cursor_advance() {
    let run_id = TurnRunId::new();
    let turns = Arc::new(EventTurnCoordinator::new(event_run_state(
        run_id,
        ProductConversationRouteKind::Direct,
    )));
    let completed = turns.transition(TurnStatus::Completed, None, 2);
    let completed_event =
        TurnLifecycleEvent::from_run_state(&completed, TurnEventKind::Completed, None);
    let event_log = Arc::new(StaticDurableTurnEventLog::new(vec![
        completed_event.clone(),
    ]));
    let binding = Arc::new(EffectiveMembershipBindingService::new(
        ProductConversationRouteKind::Direct,
    ));
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let threads = Arc::new(InMemorySessionThreadService::default());
    seed_final_message(&threads, run_id, "exactly-once final").await;
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let handler = build_event_delivery_handler(
        Arc::clone(&turns),
        Arc::clone(&binding),
        Arc::clone(&adapter),
        Arc::clone(&threads),
        Arc::clone(&store),
        None,
    );
    let router = RunDeliveryEventRouter::new(
        Arc::clone(&event_log) as Arc<dyn TurnEventProjectionSource>,
        run_state_store(Arc::clone(&turns) as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    router.register(EXTENSION_ID, &handler);
    router.wait_until_durable_replay_idle().await;
    assert_eq!(adapter.texts(), vec!["exactly-once final"]);

    // Simulate a later post-Completed cursor advance in live run state, then a
    // recovery process re-encountering the same rebuildable handoff (frozen at
    // event cursor 2). The durable Final at-most-once identity must key off the
    // frozen event cursor the drain validated, not the re-fetched live
    // `state.event_cursor`; otherwise the deterministic delivery attempt lands
    // on a fresh id and the terminal first send fails to suppress a duplicate.
    turns.transition(TurnStatus::Completed, None, 7);
    store
        .put_run_final_reply_handoff(ironclaw_outbound::RunFinalReplyHandoffRecord {
            event_cursor: completed_event.cursor,
            scope: completed_event.scope.clone(),
            run_id,
        })
        .await
        .expect("reinsert handoff for recovery");
    let recovery_handler = build_event_delivery_handler(
        Arc::clone(&turns),
        binding,
        Arc::clone(&adapter),
        threads,
        Arc::clone(&store),
        None,
    );
    let recovery_router = RunDeliveryEventRouter::new(
        event_log as Arc<dyn TurnEventProjectionSource>,
        run_state_store(turns as Arc<dyn TurnCoordinator>),
        Arc::clone(&store) as Arc<dyn OutboundStateStore>,
    );
    recovery_router.register(EXTENSION_ID, &recovery_handler);
    recovery_router.wait_until_durable_replay_idle().await;
    assert_eq!(
        adapter.texts(),
        vec!["exactly-once final"],
        "the final reply must dedup on the frozen event cursor even after a later cursor advance"
    );
}

#[tokio::test]
async fn accepted_user_message_reconciles_lifecycle_event_after_source_route_commit() {
    let harness = build_event_delivery_harness(
        Some("https://auth.example/connect"),
        ProductConversationRouteKind::Direct,
    );
    harness.binding.remove_personal_membership();
    let blocked = harness
        .turns
        .transition(TurnStatus::BlockedAuth, Some("auth:post-admission"), 2);

    // The coordinator can publish before the inbound workflow's final source
    // route commit becomes visible. That attempt must remain retryable.
    harness.publish(blocked, TurnEventKind::Blocked).await;
    assert!(harness.adapter.texts().is_empty());

    harness.binding.authorize();
    harness
        ._handler
        .reconcile_accepted_user_message(
            harness.router.as_ref(),
            &user_message_envelope(ProductTriggerReason::DirectChat, "evt:post-admission"),
            &accepted_ack(harness.run_id),
        )
        .await
        .expect("post-admission reconciliation");
    harness.router.wait_until_run_idle(harness.run_id).await;

    assert_eq!(
        harness.adapter.texts(),
        vec![
            "Authentication required\n\nAuthenticate to continue this run.\n\nReply `auth deny auth:post-admission` here to cancel this run.\n\nSetup link: https://auth.example/connect"
                .to_string(),
        ]
    );
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

#[tokio::test]
async fn triggered_non_serviceable_typed_auth_cancels_with_exact_safe_notice() {
    const GENERIC_NOTICE: &str = "This authentication step can't be completed in chat. Open the Ironclaw web app to review it, then ask me again here.";
    const MANUAL_TOKEN_NOTICE: &str = "Setting this up needs a credential (an API key or token). Sharing one here is a security risk — anything entered in chat is stored in the conversation — so credential-based connections can only be set up in the Ironclaw web app. Connect it there, then ask me again here.";
    const TRIGGER_FOOTER: &str = "\n\n_From a triggered event: “watch the deploys”. You can't interact with triggered events here — open the Ironclaw web app to interact with this run._";
    const PRIVATE_PROMPT_MATERIAL: &str = "private-prompt-material-must-not-be-echoed";

    for (challenge_kind, expected_notice) in [
        (AuthPromptChallengeKind::ManualToken, MANUAL_TOKEN_NOTICE),
        (AuthPromptChallengeKind::Other, GENERIC_NOTICE),
        (AuthPromptChallengeKind::Pairing, GENERIC_NOTICE),
        (AuthPromptChallengeKind::OAuthUrl, GENERIC_NOTICE),
    ] {
        let harness = build_triggered_harness_with_prompt(
            vec![scripted_state(
                TurnStatus::BlockedAuth,
                Some("gate:auth-private"),
            )],
            Some(Arc::new(StaticAuthPromptSource {
                challenge_kind,
                authorization_url: None,
                pairing: None,
                body_override: Some(PRIVATE_PROMPT_MATERIAL.to_string()),
            })),
            true,
        );
        seed_preference(&harness.store).await;
        let run_id = TurnRunId::new();

        harness
            .driver
            .on_trigger_submitted(triggered_request(run_id, false))
            .await;

        assert_eq!(
            wait_for_outcome(&harness.delivery_store, run_id).await,
            TriggeredRunDeliveryOutcomeKind::Delivered
        );
        assert_eq!(
            harness.turns.cancel_call_count(),
            1,
            "{challenge_kind:?} must not leave the triggered run blocked"
        );
        assert_eq!(
            harness.adapter.texts(),
            vec![format!("{expected_notice}{TRIGGER_FOOTER}")],
            "{challenge_kind:?} must deliver only its terminal-safe WebUI notice"
        );
        assert!(
            !harness.adapter.texts()[0].contains(PRIVATE_PROMPT_MATERIAL),
            "{challenge_kind:?} must not echo prompt or credential material"
        );
    }
}
