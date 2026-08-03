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
use ironclaw_event_projections::ProjectionCursor as OutboundProjectionCursor;
use ironclaw_extension_contracts::channel_adapter::ChannelAdapter;
use ironclaw_extension_contracts::preference_target::{
    PreferenceTargetCodec, PreferenceTargetEncodeRequest,
};
use ironclaw_host_api::turn::{
    AcceptedMessageRef, EventCursor, ReplyTargetBindingRef, RunProfileId, RunProfileVersion,
    SourceBindingRef, TurnGateRef, TurnId, TurnRunId, TurnScope, TurnStatus,
};
use ironclaw_host_api::{
    attachment::WorkspaceFile,
    ids::{AgentId, ExtensionId, TenantId, ThreadId, UserId},
    path::ScopedPath,
};
use ironclaw_outbound::{
    AdvanceSubscriptionCursorRequest, ClaimDeliveryAttemptForSendOutcome,
    ClaimDeliveryAttemptForSendRequest, CommunicationModality, CommunicationPreferenceRecord,
    CommunicationPreferenceRepository, DeliveredGateRouteStore, DeliveryDefaultScope,
    DeliveryFailureKind, FailPreparedDeliveryAttemptOutcome, FailPreparedDeliveryAttemptRequest,
    LoadSubscriptionCursorRequest, OutboundDeliveryAttempt, OutboundDeliveryId,
    OutboundDeliveryStatus, OutboundError, OutboundPushPlan, OutboundPushTargetRequest,
    OutboundStateStore, OutboundStateStorePort, ProjectionSubscriptionRecord,
    RecoverInterruptedDeliveryRequest, RunDeliveryCleanupRecord, RunDeliveryCleanupRequest,
    RunFinalReplyHandoffRecord, RunFinalReplyTargetRecord, RunFinalReplyTargetRequest,
    ThreadNotificationPolicy, TriggerCommunicationContext, TriggerFireSlot, TriggerOriginRef,
    TriggerSourceKind, TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryStore,
    UpdateDeliveryStatusRequest,
};
use ironclaw_product::{
    AdapterInstallationId, AuthPromptView, AuthRequirement, ChannelError, DeliveryReport,
    ExternalActorRef, ExternalConversationRef, ExternalEventId, InboundCommandPayload,
    InboundOutcome, OutboundEnvelope, OutboundPart, ParsedProductInbound, PartDeliveryOutcome,
    ProductAdapterError, ProductAdapterId, ProductCommandResultPayload, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductRejection, ProductRejectionKind,
    ProductTriggerReason, ProtocolAuthEvidence, TrustedInboundContext, UserMessagePayload,
    VerifiedInbound,
};
use ironclaw_product::{
    DeliveryCoordinator, DeliveryRetryPolicy, RunDeliveryObserver, RunDeliveryServices,
    RunDeliverySettings, TriggeredRunDeliveryDriver, TriggeredRunDeliveryRequest,
};
use ironclaw_product::{
    ProjectFilesystemReader, ProjectFsEntry, ProjectFsEntryKind, ProjectFsError, ProjectFsStat,
};
use ironclaw_product_contracts::account_setup::ChannelConnectionNoticePolicy;
use ironclaw_product_contracts::delivery::{
    ChannelDeliveryResolver, DeliveryReplyContextSource, ResolvedChannelDelivery,
};
use ironclaw_product_contracts::prompt_source::{
    BlockedAuthPromptRequest, BlockedAuthPromptSource,
};
use ironclaw_threads::{
    AppendFinalizedAssistantMessageRequest, AttachmentKind, AttachmentRef, EnsureThreadRequest,
    InMemorySessionThreadService, MessageContent, SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, GetRunStateRequest, ResumeTurnRequest, ResumeTurnResponse,
    RetryTurnRequest, RetryTurnResponse, SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator,
    TurnError, TurnRunState,
};

// ── Scripted fakes ─────────────────────────────────────────────────────────

const DELIVERY_ERROR_FEEDBACK_TEXT: &str =
    "Something went wrong delivering the result here. Check the WebUI.";

#[derive(Clone)]
struct ScriptedRunState {
    status: TurnStatus,
    gate_ref: Option<TurnGateRef>,
}

fn scripted_state(status: TurnStatus, gate_ref: Option<&str>) -> ScriptedRunState {
    ScriptedRunState {
        status,
        gate_ref: gate_ref.map(|s| TurnGateRef::new(s).expect("gate ref")),
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
            allow_steering: true,
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
        _egress: &dyn ironclaw_extension_contracts::tool_adapter::RestrictedEgress,
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
impl ironclaw_extension_contracts::tool_adapter::RestrictedEgress for DenyAllEgress {
    async fn send(
        &self,
        _request: ironclaw_extension_contracts::tool_adapter::RestrictedEgressRequest,
    ) -> Result<
        ironclaw_extension_contracts::tool_adapter::RestrictedEgressResponse,
        ironclaw_extension_contracts::tool_adapter::RestrictedEgressError,
    > {
        Err(ironclaw_extension_contracts::tool_adapter::RestrictedEgressError::PolicyDenied)
    }
}

struct StaticResolver {
    adapter: Arc<RecordingChannelAdapter>,
}

impl ChannelDeliveryResolver for StaticResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        Some(ResolvedChannelDelivery {
            extension_id: ExtensionId::new(extension_id).expect("valid extension id"),
            installation_id: AdapterInstallationId::new("install_alpha")
                .expect("valid installation id"),
            adapter: Arc::clone(&self.adapter) as Arc<dyn ChannelAdapter>,
            egress: Arc::new(DenyAllEgress),
        })
    }
}

struct UnavailableResolver;

impl ChannelDeliveryResolver for UnavailableResolver {
    fn resolve_channel_delivery(&self, _extension_id: &str) -> Option<ResolvedChannelDelivery> {
        None
    }
}

struct NoStoredReplyContext;

#[async_trait]
impl DeliveryReplyContextSource for NoStoredReplyContext {
    async fn reply_context(
        &self,
        _: &ExtensionId,
        _: &AdapterInstallationId,
        _: &str,
    ) -> Option<Vec<u8>> {
        None
    }
}

/// Forces the coordinator to lose its egress claim after a competing writer
/// has durably moved the same attempt to `observed_status`. The production
/// caller still drives the real policy/coordinator path; only the exact
/// cross-worker race is controlled here.
struct ClaimLossStore {
    inner: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    observed_status: OutboundDeliveryStatus,
    failure_kind: Option<DeliveryFailureKind>,
    run_notification_claim_calls: Arc<AtomicUsize>,
    claim_loss_projection_prefix: &'static str,
    targeted_claim_calls: AtomicUsize,
    authoritative_attempts: Mutex<HashMap<OutboundDeliveryId, OutboundDeliveryAttempt>>,
    list_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl OutboundStateStorePort for ClaimLossStore {
    async fn put_run_delivery_cleanup(
        &self,
        record: RunDeliveryCleanupRecord,
    ) -> Result<(), OutboundError> {
        self.inner.put_run_delivery_cleanup(record).await
    }

    async fn load_run_delivery_cleanup(
        &self,
        request: RunDeliveryCleanupRequest,
    ) -> Result<Vec<RunDeliveryCleanupRecord>, OutboundError> {
        self.inner.load_run_delivery_cleanup(request).await
    }

    async fn complete_run_delivery_cleanup(
        &self,
        record: &RunDeliveryCleanupRecord,
    ) -> Result<(), OutboundError> {
        self.inner.complete_run_delivery_cleanup(record).await
    }

    async fn put_run_final_reply_handoff(
        &self,
        record: RunFinalReplyHandoffRecord,
    ) -> Result<(), OutboundError> {
        self.inner.put_run_final_reply_handoff(record).await
    }

    async fn list_pending_run_final_reply_handoffs(
        &self,
        limit: usize,
    ) -> Result<Vec<RunFinalReplyHandoffRecord>, OutboundError> {
        self.inner
            .list_pending_run_final_reply_handoffs(limit)
            .await
    }

    async fn list_pending_run_final_reply_handoffs_after(
        &self,
        after: Option<&RunFinalReplyHandoffRecord>,
        limit: usize,
    ) -> Result<Vec<RunFinalReplyHandoffRecord>, OutboundError> {
        self.inner
            .list_pending_run_final_reply_handoffs_after(after, limit)
            .await
    }

    async fn complete_run_final_reply_handoff(
        &self,
        record: &RunFinalReplyHandoffRecord,
    ) -> Result<(), OutboundError> {
        self.inner.complete_run_final_reply_handoff(record).await
    }

    async fn load_run_final_reply_handoff_cursor(&self) -> Result<EventCursor, OutboundError> {
        self.inner.load_run_final_reply_handoff_cursor().await
    }

    async fn advance_run_final_reply_handoff_cursor(
        &self,
        cursor: EventCursor,
    ) -> Result<(), OutboundError> {
        self.inner
            .advance_run_final_reply_handoff_cursor(cursor)
            .await
    }

    async fn put_run_final_reply_target(
        &self,
        record: RunFinalReplyTargetRecord,
    ) -> Result<(), OutboundError> {
        self.inner.put_run_final_reply_target(record).await
    }

    async fn load_run_final_reply_target(
        &self,
        request: RunFinalReplyTargetRequest,
    ) -> Result<Option<RunFinalReplyTargetRecord>, OutboundError> {
        self.inner.load_run_final_reply_target(request).await
    }

    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        self.inner.put_thread_notification_policy(policy).await
    }

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        self.inner.load_thread_notification_policy(scope).await
    }

    async fn plan_push_targets(
        &self,
        request: OutboundPushTargetRequest,
    ) -> Result<OutboundPushPlan, OutboundError> {
        self.inner.plan_push_targets(request).await
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        self.inner.upsert_subscription(record).await
    }

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<OutboundProjectionCursor>, OutboundError> {
        self.inner.load_subscription_cursor(request).await
    }

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        self.inner.advance_subscription_cursor(request).await
    }

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        self.authoritative_attempts
            .lock()
            .expect("authoritative attempts")
            .entry(attempt.delivery_id)
            .or_insert_with(|| attempt.clone());
        self.inner.record_delivery_attempt(attempt).await
    }

    async fn claim_delivery_attempt_for_send(
        &self,
        request: ClaimDeliveryAttemptForSendRequest,
    ) -> Result<ClaimDeliveryAttemptForSendOutcome, OutboundError> {
        let projection_ref = self
            .authoritative_attempts
            .lock()
            .expect("authoritative attempts")
            .get(&request.delivery_id)
            .expect("claim-loss attempt was recorded first")
            .candidate
            .projection_ref
            .as_str()
            .to_string();
        // Failure feedback is a separate system-notice delivery. Count and intercept only the
        // policy/run-notification family selected by the fixture, so feedback still exercises
        // the real Prepared -> Sending -> Delivered path.
        if projection_ref.starts_with("run-notification:") {
            self.run_notification_claim_calls
                .fetch_add(1, Ordering::SeqCst);
        }
        // Keep the targeted counter after this short-circuit so other families do not increment it.
        if !projection_ref.starts_with(self.claim_loss_projection_prefix)
            || self.targeted_claim_calls.fetch_add(1, Ordering::SeqCst) != 0
        {
            return self.inner.claim_delivery_attempt_for_send(request).await;
        }
        let authoritative = {
            let mut attempts = self
                .authoritative_attempts
                .lock()
                .expect("authoritative attempts");
            let attempt = attempts
                .get_mut(&request.delivery_id)
                .expect("claim-loss attempt was recorded first");
            attempt.status = self.observed_status;
            attempt.failure_kind = self.failure_kind;
            attempt.clone()
        };
        self.inner
            .update_delivery_status(UpdateDeliveryStatusRequest {
                delivery_id: request.delivery_id,
                scope: request.scope,
                status: self.observed_status,
                updated_at: Utc::now(),
                failure_kind: self.failure_kind,
            })
            .await?;
        Ok(ClaimDeliveryAttemptForSendOutcome::Existing(Box::new(
            authoritative,
        )))
    }

    async fn fail_prepared_delivery_attempt(
        &self,
        request: FailPreparedDeliveryAttemptRequest,
    ) -> Result<FailPreparedDeliveryAttemptOutcome, OutboundError> {
        if self.fail_first_prepared_with_backend
            && self.fail_prepared_calls.fetch_add(1, Ordering::SeqCst) == 0
        {
            return Err(OutboundError::Backend);
        }
        self.inner.fail_prepared_delivery_attempt(request).await
    }

    async fn recover_interrupted_delivery_attempt(
        &self,
        request: RecoverInterruptedDeliveryRequest,
    ) -> Result<bool, OutboundError> {
        self.inner
            .recover_interrupted_delivery_attempt(request)
            .await
    }

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        if let Some(attempt) = self
            .authoritative_attempts
            .lock()
            .expect("authoritative attempts")
            .get_mut(&request.delivery_id)
        {
            attempt.status = request.status;
            attempt.failure_kind = request.failure_kind;
        }
        self.inner.update_delivery_status(request).await
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.list_delivery_attempts(scope).await
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
    binding: ironclaw_product::ResolvedBinding,
    fail: bool,
}

#[async_trait]
impl ironclaw_product::ConversationBindingService for StaticBindingService {
    async fn resolve_binding(
        &self,
        _request: ironclaw_product::ResolveBindingRequest,
    ) -> Result<ironclaw_product::ResolvedBinding, ironclaw_product::ProductSurfaceFailure> {
        if self.fail {
            return Err(
                ironclaw_product::ProductSurfaceFailure::BindingResolutionFailed {
                    reason: "unbound".to_string(),
                },
            );
        }
        Ok(self.binding.clone())
    }

    async fn lookup_binding(
        &self,
        _request: ironclaw_product::ResolveBindingRequest,
    ) -> Result<ironclaw_product::ResolvedBinding, ironclaw_product::ProductSurfaceFailure> {
        if self.fail {
            return Err(
                ironclaw_product::ProductSurfaceFailure::BindingResolutionFailed {
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
            auth_request_ref: request.gate_ref.as_str().to_string(),
            invocation_id: None,
            headline: "Authentication required".to_string(),
            body: request.body,
            challenge_kind: None,
            provider: None,
            account_label: None,
            authorization_url: self.authorization_url.clone(),
            expires_at: None,
            connection: None,
            pairing: None,
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
        _request: PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef> {
        None
    }

    fn encode_personal_direct_message_target(
        &self,
        _request: PreferenceTargetEncodeRequest<'_>,
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

fn binding() -> ironclaw_product::ResolvedBinding {
    ironclaw_product::ResolvedBinding {
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
    envelope_for_conversation_replying_to(payload, event_id, conversation_id, None, None)
}

fn envelope_for_conversation_replying_to(
    payload: ProductInboundPayload,
    event_id: &str,
    conversation_id: &str,
    topic_id: Option<&str>,
    reply_target_message_id: Option<&str>,
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
        ExternalConversationRef::new(
            Some("space-1"),
            conversation_id,
            topic_id,
            reply_target_message_id,
        )
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
    store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    route_store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
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
    build_harness_with_commands(states, bind_fails, auth_url, max_wait, &["status"], None)
}

/// Same as `build_harness`, but with an explicit declared-command set for the
/// observer's static help text (`build_harness` always enables `["status"]`
/// with no display prefix).
fn build_harness_with_commands(
    states: Vec<ScriptedRunState>,
    bind_fails: bool,
    auth_url: Option<&str>,
    max_wait: Duration,
    commands: &[&str],
    prefix: Option<&str>,
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
        commands,
        prefix,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_harness_with_settings(
    states: Vec<ScriptedRunState>,
    bind_fails: bool,
    auth_url: Option<&str>,
    settings: RunDeliverySettings,
    commands: &[&str],
    prefix: Option<&str>,
) -> Harness {
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let route_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let turns = Arc::new(ScriptedTurnCoordinator::with_states(states));
    let threads = Arc::new(InMemorySessionThreadService::default());
    let project_files = Arc::new(ScriptedProjectFilesystemReader::default());
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn OutboundStateStorePort>,
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
        outbound_store: Arc::clone(&store) as Arc<dyn OutboundStateStorePort>,
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
    let observer = Arc::new(
        RunDeliveryObserver::with_settings_and_connection_notices(
            services,
            settings,
            connection_notices.clone(),
        )
        .with_enabled_commands(commands.iter().copied(), prefix),
    );
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

fn build_claim_loss_observer_harness(
    states: Vec<ScriptedRunState>,
    observed_status: OutboundDeliveryStatus,
    failure_kind: Option<DeliveryFailureKind>,
    claim_loss_projection_prefix: &'static str,
    auth_url: Option<&str>,
) -> (Harness, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let route_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let turns = Arc::new(ScriptedTurnCoordinator::with_states(states));
    let threads = Arc::new(InMemorySessionThreadService::default());
    let project_files = Arc::new(ScriptedProjectFilesystemReader::default());
    let run_notification_claim_calls = Arc::new(AtomicUsize::new(0));
    let list_calls = Arc::new(AtomicUsize::new(0));
    let claim_loss_store = Arc::new(ClaimLossStore {
        inner: Arc::clone(&store),
        observed_status,
        failure_kind,
        run_notification_claim_calls: Arc::clone(&run_notification_claim_calls),
        claim_loss_projection_prefix,
        targeted_claim_calls: AtomicUsize::new(0),
        authoritative_attempts: Mutex::new(HashMap::new()),
        list_calls: Arc::clone(&list_calls),
    });
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&claim_loss_store) as Arc<dyn OutboundStateStorePort>,
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
            fail: false,
        }),
        thread_service: Arc::clone(&threads) as Arc<dyn SessionThreadService>,
        turn_coordinator: Arc::clone(&turns) as Arc<dyn TurnCoordinator>,
        outbound_store: claim_loss_store as Arc<dyn OutboundStateStorePort>,
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
        RunDeliverySettings {
            poll_interval: Duration::from_millis(1),
            max_wait: Duration::from_millis(15),
            max_concurrent_deliveries: NonZeroUsize::new(4).expect("nz"),
            max_pending_deliveries: NonZeroUsize::new(8).expect("nz"),
        },
        connection_notices.clone(),
    ));
    let harness = Harness {
        observer,
        connection_notices,
        adapter,
        store,
        route_store,
        turns,
        threads,
        project_files,
    };
    (harness, run_notification_claim_calls, list_calls)
}

async fn seed_final_message(threads: &InMemorySessionThreadService, run_id: TurnRunId, text: &str) {
    seed_final_message_with_attachments(threads, run_id, text, Vec::new()).await;
}

async fn seed_final_message_with_attachments(
    threads: &InMemorySessionThreadService,
    run_id: TurnRunId,
    text: &str,
    attachments: Vec<AttachmentRef>,
) {
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
            content: MessageContent::with_attachments(text, attachments),
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
}

#[tokio::test]
async fn observer_materializes_finalized_attachment_refs_for_delivery() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        false,
        None,
        Duration::from_secs(5),
    );
    let run_id = TurnRunId::new();
    harness
        .project_files
        .insert_file("/workspace/report.txt", "text/plain", b"hello");
    seed_final_message_with_attachments(
        &harness.threads,
        run_id,
        "report attached",
        vec![AttachmentRef {
            id: "reply-attachment-0".to_string(),
            kind: AttachmentKind::Document,
            mime_type: "text/plain".to_string(),
            filename: Some("renamed-report.txt".to_string()),
            size_bytes: Some(5),
            storage_key: Some("/workspace/report.txt".to_string()),
            extracted_text: None,
        }],
    )
    .await;

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-file-final"),
            accepted_ack(run_id),
        )
        .await;

    let envelopes = harness.adapter.envelopes();
    assert_eq!(envelopes.len(), 1);
    assert!(matches!(
        envelopes[0].parts.as_slice(),
        [OutboundPart::Text(text), OutboundPart::File(file)]
            if text == "report attached"
                && file.path.as_str() == "/workspace/report.txt"
                && file.filename.as_deref() == Some("renamed-report.txt")
                && file.mime_type == "text/plain"
                && file.bytes == b"hello"
    ));
}

#[tokio::test]
async fn observer_delivers_command_result_through_the_coordinator() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        false,
        None,
        Duration::from_secs(5),
    );
    let command =
        InboundCommandPayload::new("model", "", ProductTriggerReason::BotCommand).expect("command");
    let command_envelope = envelope(
        ProductInboundPayload::Command(command),
        "evt-command-result",
    );

    harness
        .observer
        .observe_ack(
            command_envelope,
            ProductInboundAck::CommandResult {
                command: "model".to_string(),
                payload: ProductCommandResultPayload::new(serde_json::json!({
                    "active": {
                        "model": "gpt-5.5"
                    },
                    "configured": true
                })),
            },
        )
        .await;

    assert_eq!(
        harness.adapter.texts(),
        vec![
            "Command `/model` completed.\n\n    {\n      \"active\": {\n        \"model\": \"gpt-5.5\"\n      },\n      \"configured\": true\n    }"
                .to_string()
        ]
    );
    let envelopes = harness.adapter.envelopes();
    assert_eq!(envelopes.len(), 1);
    assert_eq!(envelopes[0].target.conversation.conversation_id(), "conv-1");
    assert_eq!(envelopes[0].extension_id, EXTENSION_ID);
}

#[tokio::test]
async fn observer_delivers_scoped_command_help_for_invalid_request() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        false,
        None,
        Duration::from_secs(5),
    );
    let command = InboundCommandPayload::new("notacommand", "", ProductTriggerReason::DirectChat)
        .expect("command");
    let command_envelope = envelope(
        ProductInboundPayload::Command(command),
        "evt-command-invalid",
    );

    harness
        .observer
        .observe_ack(
            command_envelope,
            ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::InvalidRequest,
                "opaque parser or admission detail",
            )),
        )
        .await;

    assert_eq!(
        harness.adapter.texts(),
        vec!["Available commands:\n/status".to_string()]
    );
}

#[tokio::test]
async fn access_denied_command_rejection_delivers_admin_notice() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        false,
        None,
        Duration::from_secs(5),
    );
    let command =
        InboundCommandPayload::new("extension_configure", "", ProductTriggerReason::DirectChat)
            .expect("command");
    let command_envelope = envelope(
        ProductInboundPayload::Command(command),
        "evt-command-access-denied",
    );
    let internal_reason = "admin-audience command from a non-admin actor";

    harness
        .observer
        .observe_ack(
            command_envelope,
            ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::AccessDenied,
                internal_reason,
            )),
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(
        texts,
        vec!["This command requires an admin account.".to_string()]
    );
    assert!(
        texts.iter().all(|text| !text.contains(internal_reason)),
        "the internal rejection reason must never reach the delivered text: {texts:?}"
    );
}

#[tokio::test]
async fn static_command_help_excludes_admin_audience_commands() {
    let harness = build_harness_with_commands(
        vec![scripted_state(TurnStatus::Completed, None)],
        false,
        None,
        Duration::from_secs(5),
        &["model", "status", "extension_configure"],
        None,
    );
    let command = InboundCommandPayload::new("notacommand", "", ProductTriggerReason::DirectChat)
        .expect("command");
    let command_envelope = envelope(
        ProductInboundPayload::Command(command),
        "evt-command-invalid-role-filtered-help",
    );

    harness
        .observer
        .observe_ack(
            command_envelope,
            ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::InvalidRequest,
                "opaque parser or admission detail",
            )),
        )
        .await;

    assert_eq!(
        harness.adapter.texts(),
        vec!["Available commands:\n/model\n/status".to_string()]
    );
}

#[tokio::test]
async fn static_command_help_renders_with_manifest_declared_prefix() {
    let harness = build_harness_with_commands(
        vec![scripted_state(TurnStatus::Completed, None)],
        false,
        None,
        Duration::from_secs(5),
        &["model", "status"],
        Some("/ironclaw "),
    );
    let command = InboundCommandPayload::new("notacommand", "", ProductTriggerReason::DirectChat)
        .expect("command");
    let command_envelope = envelope(
        ProductInboundPayload::Command(command),
        "evt-command-invalid-prefixed-help",
    );

    harness
        .observer
        .observe_ack(
            command_envelope,
            ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::InvalidRequest,
                "opaque parser or admission detail",
            )),
        )
        .await;

    assert_eq!(
        harness.adapter.texts(),
        vec!["Available commands:\n/ironclaw model\n/ironclaw status".to_string()]
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
            ironclaw_product::ApprovalResolutionPayload::new(
                "gate-1",
                ironclaw_product::ApprovalDecision::ApproveOnce,
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
    let harness = build_harness_with_settings(states, false, None, settings, &["status"], None);
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

    // A *threaded* prompting event that is itself a reply. Both halves are
    // load-bearing:
    //
    // - the topic (`1700.1`) makes the source branch's key distinguishable from
    //   the delivered-message loop's, which only ever keys the topic off a
    //   vendor message ref (`ts-N` here) or leaves it empty. Without a topic the
    //   two branches produce the same conversation-root key and an assertion on
    //   it passes no matter what the source branch does.
    // - the reply target (`1800.2`) is the per-event id the recorded route must
    //   NOT inherit: a later bare `approve` in the same topic carries a
    //   different one (or none), and a key that varied with it would never match.
    harness
        .observer
        .observe_ack(
            envelope_for_conversation_replying_to(
                ProductInboundPayload::UserMessage(
                    UserMessagePayload::new("hello", Vec::new(), ProductTriggerReason::DirectChat)
                        .expect("payload"),
                ),
                "evt-gate",
                "conv-1",
                Some("1700.1"),
                Some("1800.2"),
            ),
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
    // The source topic (bare replies next to the prompt) routes too, keyed by
    // the topic and WITHOUT the prompting event's reply target. Only the source
    // branch can produce this key — the delivered loop's topics are vendor
    // message refs.
    let source_fingerprint =
        ExternalConversationRef::new(Some("space-1"), "conv-1", Some("1700.1"), None)
            .expect("conversation")
            .conversation_fingerprint();
    // Non-vacuity for the membership check below: if the topic did NOT
    // participate in the fingerprint, `source_fingerprint` would just be the
    // untargeted conversation's key, which the route records anyway — so the
    // assertion would pass while proving nothing about topic routing.
    assert_ne!(
        source_fingerprint,
        ExternalConversationRef::new(Some("space-1"), "conv-1", None, None)
            .expect("conversation")
            .conversation_fingerprint(),
        "the conversation topic must participate in the fingerprint"
    );
    assert!(
        route
            .delivered_conversation_fingerprints
            .contains(&source_fingerprint),
        "a gate route recorded from a threaded reply must still be resolvable by a \
         bare reply in the same topic that carries no reply target: {:?}",
        route.delivered_conversation_fingerprints
    );
    // The invariant the assertion above leans on, pinned here rather than left
    // to be re-derived from `conversation_fingerprint`'s body: the fingerprint
    // is the ROUTE, so it does not vary with the per-event reply target. If that
    // ever stopped holding, the recording branch would start baking a message id
    // into a stable key and the failure above would look unrelated to the cause.
    assert_eq!(
        ExternalConversationRef::new(Some("space-1"), "conv-1", Some("1700.1"), Some("1800.2"))
            .expect("conversation")
            .conversation_fingerprint(),
        source_fingerprint,
        "conversation_fingerprint must exclude the reply-target hint"
    );
}

#[tokio::test]
async fn observer_claim_loser_never_publishes_a_gate_route_without_confirmed_delivery() {
    const GATE_REF: &str = "gate:claim-loser-approval";
    let unconfirmed_states = [
        (OutboundDeliveryStatus::Sending, None),
        (OutboundDeliveryStatus::Unknown, None),
        (OutboundDeliveryStatus::Pending, None),
        (
            OutboundDeliveryStatus::Failed,
            Some(DeliveryFailureKind::Rejected),
        ),
        (
            OutboundDeliveryStatus::DeadLettered,
            Some(DeliveryFailureKind::TransportUnavailable),
        ),
    ];

    for (index, (status, failure_kind)) in unconfirmed_states.into_iter().enumerate() {
        let (harness, _, list_calls) = build_claim_loss_observer_harness(
            vec![scripted_state(TurnStatus::BlockedApproval, Some(GATE_REF))],
            status,
            failure_kind,
            "run-notification:approval:",
            None,
        );
        let run_id = TurnRunId::new();

        harness
            .observer
            .observe_ack(
                user_message_envelope(
                    ProductTriggerReason::DirectChat,
                    &format!("evt-claim-loser-gate-{index}"),
                ),
                accepted_ack(run_id),
            )
            .await;

        assert_eq!(
            list_calls.load(Ordering::SeqCst),
            1,
            "the coordinator must recover the scope once and consume the authoritative claim outcome"
        );
        let texts = harness.adapter.texts();
        assert_eq!(
            texts,
            Vec::<String>::new(),
            "a {status:?} approval claim loser must not emit the prompt or failure feedback"
        );
        assert!(
            harness.adapter.envelopes().is_empty(),
            "a {status:?} approval claim loser must produce zero adapter envelopes"
        );
        assert!(
            harness
                .route_store
                .load_delivered_gate_route(&tenant(), &user(), GATE_REF)
                .await
                .expect("route lookup")
                .is_none(),
            "a {status:?} row is not evidence that this approval prompt was delivered"
        );
        let attempts = harness
            .store
            .list_delivery_attempts(binding_scope())
            .await
            .expect("attempts");
        assert_eq!(
            attempts.len(),
            1,
            "claim loss must not persist a separate FailureNotice attempt"
        );
        assert_eq!(attempts[0].status, status);
        assert_eq!(attempts[0].failure_kind, failure_kind);
        assert!(
            attempts[0]
                .candidate
                .projection_ref
                .as_str()
                .starts_with("run-notification:approval:"),
            "the sole durable row must be the original unconfirmed gate notification"
        );
    }
}

#[tokio::test]
async fn observer_only_a_durable_delivered_claim_loss_may_publish_the_gate_route() {
    const GATE_REF: &str = "gate:confirmed-duplicate-approval";
    let (harness, _, list_calls) = build_claim_loss_observer_harness(
        vec![scripted_state(TurnStatus::BlockedApproval, Some(GATE_REF))],
        OutboundDeliveryStatus::Delivered,
        None,
        "run-notification:approval:",
        None,
    );

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-confirmed-duplicate"),
            accepted_ack(TurnRunId::new()),
        )
        .await;

    assert_eq!(
        list_calls.load(Ordering::SeqCst),
        1,
        "the coordinator must recover the scope once and consume the authoritative claim outcome"
    );
    assert!(harness.adapter.envelopes().is_empty());
    assert!(
        harness
            .route_store
            .load_delivered_gate_route(&tenant(), &user(), GATE_REF)
            .await
            .expect("route lookup")
            .is_some(),
        "observed durable Delivered is the sole claim-loss state that confirms delivery"
    );
}

#[tokio::test]
async fn observer_unconfirmed_terminal_claim_loss_does_not_poison_the_delivered_run_ledger() {
    let (harness, run_notification_claim_calls, list_calls) = build_claim_loss_observer_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        OutboundDeliveryStatus::Unknown,
        None,
        "run-notification:final:",
        None,
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "truthful final reply").await;

    for event_id in ["evt-terminal-claim-loser-a", "evt-terminal-claim-loser-b"] {
        harness
            .observer
            .observe_ack(
                user_message_envelope(ProductTriggerReason::DirectChat, event_id),
                accepted_ack(run_id),
            )
            .await;
    }

    assert_eq!(
        list_calls.load(Ordering::SeqCst),
        1,
        "the coordinator must recover the scope once and consume each authoritative claim outcome"
    );
    assert_eq!(
        run_notification_claim_calls.load(Ordering::SeqCst),
        2,
        "an Unknown claim loser must not record the run as delivered and suppress a later replay"
    );
    assert_eq!(
        harness.adapter.texts(),
        Vec::<String>::new(),
        "repeated Unknown claim loss must not emit the final reply or failure feedback"
    );
    assert!(
        harness.adapter.envelopes().is_empty(),
        "repeated Unknown claim loss must produce zero adapter envelopes"
    );
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert_eq!(
        attempts.len(),
        1,
        "the stable final-delivery identity must be retried without persisting feedback attempts"
    );
    assert!(
        attempts.iter().all(|attempt| {
            attempt.status == OutboundDeliveryStatus::Unknown
                && attempt.failure_kind.is_none()
                && attempt
                    .candidate
                    .projection_ref
                    .as_str()
                    .starts_with("run-notification:final:")
        }),
        "the durable row must preserve the authoritative Unknown final-delivery state"
    );
}

#[tokio::test]
async fn observer_unconfirmed_terminal_claim_loss_does_not_clean_up_a_truthfully_delivered_prompt()
{
    let (harness, run_notification_claim_calls, list_calls) = build_claim_loss_observer_harness(
        vec![
            scripted_state(TurnStatus::Running, None),
            scripted_state(TurnStatus::BlockedAuth, Some("gate:auth-before-claim-loss")),
            scripted_state(TurnStatus::Completed, None),
        ],
        OutboundDeliveryStatus::Unknown,
        None,
        "run-notification:final:",
        Some("https://provider.example/oauth"),
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "final reply is still unconfirmed").await;

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-cleanup-claim-loser"),
            accepted_ack(run_id),
        )
        .await;

    assert_eq!(
        list_calls.load(Ordering::SeqCst),
        1,
        "the coordinator must recover the scope once and consume the authoritative claim outcome"
    );
    assert_eq!(
        run_notification_claim_calls.load(Ordering::SeqCst),
        2,
        "the selected policy path claims the auth prompt and then the final reply"
    );
    let texts = harness.adapter.texts();
    assert_eq!(
        texts.len(),
        1,
        "only the already-confirmed auth prompt may reach the adapter"
    );
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.contains("Authentication required"))
            .count(),
        1,
        "the delivered auth prompt remains actionable until a terminal notification is confirmed"
    );
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.as_str() == DELIVERY_ERROR_FEEDBACK_TEXT)
            .count(),
        0,
        "the unconfirmed final reply must not post delivery-error feedback"
    );
    assert!(
        !texts
            .iter()
            .any(|text| text == "final reply is still unconfirmed"),
        "the unconfirmed final reply must not reach the adapter"
    );
    assert!(
        harness.adapter.retracted_refs().is_empty(),
        "an unconfirmed terminal loser must not queue or publish stale-prompt cleanup"
    );
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert_eq!(
        attempts.len(),
        2,
        "only the confirmed auth prompt and unconfirmed final attempt may be persisted"
    );
    assert!(
        attempts.iter().any(|attempt| {
            attempt.status == OutboundDeliveryStatus::Delivered
                && attempt.failure_kind.is_none()
                && attempt
                    .candidate
                    .projection_ref
                    .as_str()
                    .starts_with("run-notification:auth:")
        }),
        "the auth prompt remains the only confirmed adapter delivery"
    );
    assert!(
        attempts.iter().any(|attempt| {
            attempt.status == OutboundDeliveryStatus::Unknown
                && attempt.failure_kind.is_none()
                && attempt
                    .candidate
                    .projection_ref
                    .as_str()
                    .starts_with("run-notification:final:")
        }),
        "the final attempt must retain its authoritative Unknown state"
    );
    assert!(
        attempts.iter().all(|attempt| !attempt
            .candidate
            .projection_ref
            .as_str()
            .starts_with("system-notice:failure-notice:")),
        "claim loss must not persist a FailureNotice attempt"
    );
}

#[tokio::test]
async fn observer_owner_terminal_delivery_failure_posts_exactly_one_failure_notice() {
    let harness = build_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        false,
        None,
        Duration::from_secs(5),
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "owner delivery will fail").await;
    harness
        .adapter
        .reports
        .lock()
        .expect("reports lock")
        .push_back(DeliveryReport {
            parts: vec![PartDeliveryOutcome::Permanent {
                reason: "scripted permanent failure".to_string(),
            }],
        });

    harness
        .observer
        .observe_ack(
            user_message_envelope(
                ProductTriggerReason::DirectChat,
                "evt-owner-terminal-failure",
            ),
            accepted_ack(run_id),
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.as_str() == DELIVERY_ERROR_FEEDBACK_TEXT)
            .count(),
        1,
        "an actual owner-side terminal failure still posts exactly one FailureNotice"
    );
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 2, "failed final plus one FailureNotice");
    assert!(
        attempts.iter().any(|attempt| {
            attempt.status == OutboundDeliveryStatus::Failed
                && attempt.failure_kind == Some(DeliveryFailureKind::Rejected)
                && attempt
                    .candidate
                    .projection_ref
                    .as_str()
                    .starts_with("run-notification:final:")
        }),
        "the owner path must reach CoordinatedDeliveryOutcome::Failed"
    );
    assert_eq!(
        attempts
            .iter()
            .filter(|attempt| attempt
                .candidate
                .projection_ref
                .as_str()
                .starts_with("system-notice:failure-notice:"))
            .count(),
        1,
        "failure feedback suppression must be specific to DeliveryUnconfirmed"
    );
}

/// A run can block on more than one approval or auth gate of the same kind.
/// The durable projection identity must therefore include the bounded gate
/// reference. The gate reference is hashed with a domain-separated,
/// length-framed SHA-256 input so the persisted metadata neither leaks the ref
/// nor admits ambiguous concatenations. Replaying the *same* gate must still
/// address the original delivery and remain duplicate-suppressed.
#[tokio::test]
async fn observer_delivers_distinct_same_kind_gates_once_each_and_suppresses_replays() {
    const APPROVAL_A: &str = "gate:approval-00000000000000000000000000000001";
    const APPROVAL_B: &str = "gate:approval-00000000000000000000000000000002";
    const AUTH_A: &str = "gate:auth-vector";
    const AUTH_B: &str = "gate:auth-vector-next";

    let harness = build_harness(
        vec![
            // The observer's existence guard consumes this first state.
            scripted_state(TurnStatus::Running, None),
            scripted_state(TurnStatus::BlockedApproval, Some(APPROVAL_A)),
            scripted_state(TurnStatus::BlockedApproval, Some(APPROVAL_B)),
            scripted_state(TurnStatus::BlockedAuth, Some(AUTH_A)),
            scripted_state(TurnStatus::BlockedAuth, Some(AUTH_B)),
            // Replays must resolve to the same identity as their first
            // appearance and therefore produce no second vendor egress.
            scripted_state(TurnStatus::BlockedApproval, Some(APPROVAL_A)),
            scripted_state(TurnStatus::BlockedAuth, Some(AUTH_A)),
            scripted_state(TurnStatus::Completed, None),
        ],
        false,
        Some("https://provider.example/oauth"),
        Duration::from_secs(5),
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "all gates resolved").await;

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-sequential-gates"),
            accepted_ack(run_id),
        )
        .await;

    let texts = harness.adapter.texts();
    assert_eq!(
        texts.len(),
        5,
        "two distinct approvals, two distinct auth gates, and one final reply must reach egress"
    );
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.contains("Approval needed"))
            .count(),
        2,
        "same-kind approval gates with distinct refs must not collide"
    );
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.contains("Authentication required"))
            .count(),
        2,
        "same-kind auth gates with distinct refs must not collide"
    );

    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert_eq!(
        attempts.len(),
        7,
        "five run notifications plus cleanup of the two delivered auth prompts"
    );
    let mut run_notification_refs: Vec<String> = attempts
        .iter()
        .map(|attempt| attempt.candidate.projection_ref.as_str())
        .filter(|projection_ref| projection_ref.starts_with("run-notification:"))
        .map(str::to_string)
        .collect();
    run_notification_refs.sort();
    let mut expected_run_notification_refs = vec![
        format!(
            "run-notification:approval:{run_id}:\
             e3206a38703ea974fc4f5902051fcf48593081aa6c025dcb21db98bf886d8c25"
        ),
        format!(
            "run-notification:approval:{run_id}:\
             78c0c16afee88400ca2434de5c1a4d975cd21733915519104f04e3d580709356"
        ),
        format!(
            "run-notification:auth:{run_id}:\
             876c0617c31155ca02b78fb0934d45d0dde148fe65fa2a11baf58eac34c72084"
        ),
        format!(
            "run-notification:auth:{run_id}:\
             62ad57165b8c00856fded1220d7cb8f785b126c640afbcb137831ab819326116"
        ),
        format!("run-notification:final:{run_id}"),
    ];
    expected_run_notification_refs.sort();
    assert_eq!(
        run_notification_refs, expected_run_notification_refs,
        "same-gate replays reuse their original rows, distinct gates own their pinned digested \
         rows, and the final identity remains byte-for-byte unchanged"
    );
    assert_eq!(
        attempts
            .iter()
            .filter(|attempt| attempt
                .candidate
                .projection_ref
                .as_str()
                .starts_with("system-notice:cleanup:retract-"))
            .count(),
        2,
        "each delivered auth prompt owns one durable cleanup/retraction row"
    );
    assert_eq!(
        harness.adapter.retracted_refs().len(),
        2,
        "both stale auth prompts are retracted after the final reply"
    );
}

async fn assert_gate_projection_digest_vector(
    status: TurnStatus,
    gate_ref: &str,
    expected_suffix: &str,
    expected_digest: &str,
) {
    let harness = build_harness(
        vec![
            scripted_state(TurnStatus::Running, None),
            scripted_state(status, Some(gate_ref)),
            scripted_state(TurnStatus::Completed, None),
        ],
        false,
        Some("https://provider.example/oauth"),
        Duration::from_secs(5),
    );
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "gate resolved").await;

    harness
        .observer
        .observe_ack(
            user_message_envelope(ProductTriggerReason::DirectChat, "evt-gate-digest-vector"),
            accepted_ack(run_id),
        )
        .await;

    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    let projection_refs: Vec<&str> = attempts
        .iter()
        .map(|attempt| attempt.candidate.projection_ref.as_str())
        .collect();
    let expected = format!("run-notification:{expected_suffix}:{run_id}:{expected_digest}");
    assert!(
        projection_refs.contains(&expected.as_str()),
        "gate identity must hash the three u64-BE-length-framed parts \
         (`ironclaw_product:run_notification_gate_projection:v1`, `{expected_suffix}`, \
         canonical gate_ref) with full lowercase SHA-256; expected {expected}, got \
         {projection_refs:?}"
    );
}

#[tokio::test]
async fn approval_gate_projection_identity_matches_the_pinned_digest_vector() {
    assert_gate_projection_digest_vector(
        TurnStatus::BlockedApproval,
        "gate:approval-00000000000000000000000000000001",
        "approval",
        "e3206a38703ea974fc4f5902051fcf48593081aa6c025dcb21db98bf886d8c25",
    )
    .await;
}

#[tokio::test]
async fn auth_gate_projection_identity_matches_the_pinned_digest_vector() {
    assert_gate_projection_digest_vector(
        TurnStatus::BlockedAuth,
        "gate:auth-vector",
        "auth",
        "876c0617c31155ca02b78fb0934d45d0dde148fe65fa2a11baf58eac34c72084",
    )
    .await;
}

#[tokio::test]
async fn observer_fails_closed_when_a_gate_kind_has_no_gate_ref() {
    for status in [TurnStatus::BlockedApproval, TurnStatus::BlockedAuth] {
        let harness = build_harness(
            vec![scripted_state(status, None)],
            false,
            Some("https://provider.example/oauth"),
            Duration::from_millis(40),
        );
        let run_id = TurnRunId::new();

        harness
            .observer
            .observe_ack(
                user_message_envelope(ProductTriggerReason::DirectChat, "evt-missing-gate-ref"),
                accepted_ack(run_id),
            )
            .await;

        assert!(
            harness.adapter.texts().is_empty(),
            "{status:?} without a gate_ref must not synthesize a weaker delivery identity"
        );
        assert!(
            harness
                .store
                .list_delivery_attempts(binding_scope())
                .await
                .expect("attempts")
                .is_empty(),
            "{status:?} without a gate_ref must fail before durable delivery preparation"
        );
    }
}

#[tokio::test]
async fn observer_records_gate_route_without_a_vendor_ref_that_cannot_key_a_route() {
    // `vendor_message_ref` is an unvalidated vendor string, so a channel can
    // hand back a ref that is not a legal route segment (here: a control
    // character). The two topic-keyed route variants must then be DROPPED
    // rather than recorded malformed -- and, crucially, the conversation-root
    // variants must still be recorded, or one bad ref would silently cost the
    // gate every route and a bare `approve` would resolve nothing.
    let harness = build_harness(
        vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some("gate:approval-00000000000000000000000000000001"),
        )],
        false,
        None,
        Duration::from_millis(40),
    );
    harness
        .adapter
        .reports
        .lock()
        .expect("reports lock")
        .push_back(DeliveryReport {
            parts: vec![PartDeliveryOutcome::Sent {
                vendor_message_ref: Some("ts-\u{7}1".to_string()),
            }],
        });
    let run_id = TurnRunId::new();

    harness
        .observer
        .observe_ack(
            envelope_for_conversation_replying_to(
                ProductInboundPayload::UserMessage(
                    UserMessagePayload::new("hello", Vec::new(), ProductTriggerReason::DirectChat)
                        .expect("payload"),
                ),
                "evt-gate-bad-ref",
                "conv-1",
                Some("1700.1"),
                None,
            ),
            accepted_ack(run_id),
        )
        .await;

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
    let fingerprint = |space: Option<&str>, topic: Option<&str>| {
        ExternalConversationRef::new(space, "conv-1", topic, None)
            .expect("conversation")
            .conversation_fingerprint()
    };
    let mut recorded = route.delivered_conversation_fingerprints.clone();
    recorded.sort();
    let mut expected = vec![
        // Delivered loop, space-qualified conversation root.
        fingerprint(Some("space-1"), None),
        // Delivered loop, no-space fallback.
        fingerprint(None, None),
        // The prompting (source) conversation, keyed by its own topic.
        fingerprint(Some("space-1"), Some("1700.1")),
    ];
    expected.sort();
    assert_eq!(
        recorded, expected,
        "an unusable vendor message ref drops only the two ref-keyed variants"
    );
    assert!(
        !recorded.iter().any(|entry| entry.contains('\u{7}')),
        "a vendor ref that is not a legal external id must never reach a route key: {recorded:?}"
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
    store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    route_store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    delivery_store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
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
        Arc::clone(&store) as Arc<dyn OutboundStateStorePort>,
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
        outbound_store: Arc::clone(&store) as Arc<dyn OutboundStateStorePort>,
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
        route_store,
        delivery_store,
        turns,
        threads,
        project_files,
    }
}

fn build_claim_loss_triggered_harness(
    state: ScriptedRunState,
    observed_status: OutboundDeliveryStatus,
    failure_kind: Option<DeliveryFailureKind>,
) -> (TriggeredHarness, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let route_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let delivery_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let turns = Arc::new(ScriptedTurnCoordinator::with_states(vec![state]));
    let threads = Arc::new(InMemorySessionThreadService::default());
    let project_files = Arc::new(ScriptedProjectFilesystemReader::default());
    let run_notification_claim_calls = Arc::new(AtomicUsize::new(0));
    let list_calls = Arc::new(AtomicUsize::new(0));
    let claim_loss_store = Arc::new(ClaimLossStore {
        inner: Arc::clone(&store),
        observed_status,
        failure_kind,
        run_notification_claim_calls: Arc::clone(&run_notification_claim_calls),
        claim_loss_projection_prefix: "run-notification:approval:",
        targeted_claim_calls: AtomicUsize::new(0),
        authoritative_attempts: Mutex::new(HashMap::new()),
        list_calls: Arc::clone(&list_calls),
    });
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&claim_loss_store) as Arc<dyn OutboundStateStorePort>,
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
        outbound_store: claim_loss_store as Arc<dyn OutboundStateStorePort>,
        route_store: Arc::clone(&route_store) as Arc<dyn DeliveredGateRouteStore>,
        communication_preferences: Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>,
        project_filesystem: Arc::clone(&project_files) as Arc<dyn ProjectFilesystemReader>,
        coordinator,
        extension_id: EXTENSION_ID.to_string(),
        fallback_notice_scope: fallback_scope(),
        approval_context: None,
        blocked_auth_prompts: None,
        auth_flow_cancel: None,
    };
    let driver = TriggeredRunDeliveryDriver::with_settings(
        services,
        RunDeliverySettings {
            poll_interval: Duration::from_millis(1),
            max_wait: Duration::from_millis(15),
            max_concurrent_deliveries: NonZeroUsize::new(4).expect("nz"),
            max_pending_deliveries: NonZeroUsize::new(8).expect("nz"),
        },
        Arc::clone(&delivery_store) as Arc<dyn TriggeredRunDeliveryStore>,
        Arc::new(StaticCodec {
            conversation: ExternalConversationRef::new(Some("space-1"), "dm-creator", None, None)
                .expect("conversation"),
            personal_dm: true,
        }),
        agent(),
    );
    (
        TriggeredHarness {
            driver,
            adapter,
            store,
            route_store,
            delivery_store,
            turns,
            threads,
            project_files,
        },
        run_notification_claim_calls,
        list_calls,
    )
}

fn build_settlement_failure_triggered_harness(
    state: ScriptedRunState,
    auth_url: Option<&str>,
    personal_dm_target: bool,
    channel_unavailable: bool,
) -> TriggeredHarness {
    let adapter = Arc::new(RecordingChannelAdapter::new());
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let route_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let delivery_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let turns = Arc::new(ScriptedTurnCoordinator::with_states(vec![state]));
    let threads = Arc::new(InMemorySessionThreadService::default());
    let project_files = Arc::new(ScriptedProjectFilesystemReader::default());
    let settlement_failure_store = Arc::new(ClaimLossStore {
        inner: Arc::clone(&store),
        observed_status: OutboundDeliveryStatus::Prepared,
        failure_kind: None,
        run_notification_claim_calls: Arc::new(AtomicUsize::new(0)),
        claim_loss_projection_prefix: "never-match:",
        targeted_claim_calls: AtomicUsize::new(0),
        authoritative_attempts: Mutex::new(HashMap::new()),
        list_calls: AtomicUsize::new(0),
        fail_first_prepared_with_backend: true,
        fail_prepared_calls: AtomicUsize::new(0),
    });
    let resolver: Arc<dyn ChannelDeliveryResolver> = if channel_unavailable {
        Arc::new(UnavailableResolver)
    } else {
        Arc::new(StaticResolver {
            adapter: Arc::clone(&adapter),
        })
    };
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&settlement_failure_store) as Arc<dyn OutboundStateStorePort>,
        resolver,
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
        outbound_store: settlement_failure_store as Arc<dyn OutboundStateStorePort>,
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
        route_store,
        delivery_store,
        turns,
        threads,
        project_files,
    }
}

async fn seed_preference(store: &OutboundStateStore<ironclaw_filesystem::InMemoryBackend>) {
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
    store: &OutboundStateStore<ironclaw_filesystem::InMemoryBackend>,
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

/// The triggered driver is a separate production caller of the projection-id
/// helper. Keep its gate reference wired into identity derivation too: a
/// trigger can park repeatedly on distinct approval/auth gates before its final
/// reply, while a replay of an already-delivered gate remains a no-op.
#[tokio::test]
async fn triggered_driver_delivers_distinct_same_kind_gates_once_each() {
    const APPROVAL_A: &str = "gate:approval-00000000000000000000000000000001";
    const APPROVAL_B: &str = "gate:approval-00000000000000000000000000000002";
    const AUTH_A: &str = "gate:auth-vector";
    const AUTH_B: &str = "gate:auth-vector-next";

    let harness = build_triggered_harness(
        vec![
            scripted_state(TurnStatus::BlockedApproval, Some(APPROVAL_A)),
            scripted_state(TurnStatus::BlockedApproval, Some(APPROVAL_B)),
            scripted_state(TurnStatus::BlockedAuth, Some(AUTH_A)),
            scripted_state(TurnStatus::BlockedAuth, Some(AUTH_B)),
            scripted_state(TurnStatus::BlockedApproval, Some(APPROVAL_A)),
            scripted_state(TurnStatus::BlockedAuth, Some(AUTH_A)),
            scripted_state(TurnStatus::Completed, None),
        ],
        Some("https://provider.example/oauth"),
        true,
    );
    seed_preference(&harness.store).await;
    // Keep this contract focused on the five run notifications. A successful
    // channel delivery may omit a vendor message ref; doing so prevents the
    // driver's terminal auth-prompt cleanup from adding unrelated Cleanup
    // rows to this projection-identity assertion.
    for _ in 0..5 {
        harness
            .adapter
            .reports
            .lock()
            .expect("reports lock")
            .push_back(DeliveryReport {
                parts: vec![PartDeliveryOutcome::Sent {
                    vendor_message_ref: None,
                }],
            });
    }
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "all triggered gates resolved").await;

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;
    assert_eq!(
        wait_for_outcome(&harness.delivery_store, run_id).await,
        TriggeredRunDeliveryOutcomeKind::Delivered
    );

    let texts = harness.adapter.texts();
    assert_eq!(
        texts.len(),
        5,
        "two distinct approvals, two distinct auth gates, and one final reply must reach the \
         triggered target"
    );
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.contains("Approval needed"))
            .count(),
        2,
        "distinct triggered approval gates must not collide"
    );
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.contains("Authentication required"))
            .count(),
        2,
        "distinct triggered auth gates must not collide"
    );
    assert!(
        harness.adapter.envelopes().iter().all(|envelope| envelope
            .target
            .conversation
            .conversation_id()
            == "dm-creator"),
        "every prompt and final reply must use the decoded personal-DM preference target"
    );

    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert_eq!(
        attempts.len(),
        5,
        "replayed gates reuse their durable rows; distinct gates do not"
    );
    let mut projection_refs: Vec<String> = attempts
        .iter()
        .map(|attempt| attempt.candidate.projection_ref.as_str().to_string())
        .collect();
    projection_refs.sort();
    let mut expected = vec![
        format!(
            "run-notification:approval:{run_id}:\
             e3206a38703ea974fc4f5902051fcf48593081aa6c025dcb21db98bf886d8c25"
        ),
        format!(
            "run-notification:approval:{run_id}:\
             78c0c16afee88400ca2434de5c1a4d975cd21733915519104f04e3d580709356"
        ),
        format!(
            "run-notification:auth:{run_id}:\
             876c0617c31155ca02b78fb0934d45d0dde148fe65fa2a11baf58eac34c72084"
        ),
        format!(
            "run-notification:auth:{run_id}:\
             62ad57165b8c00856fded1220d7cb8f785b126c640afbcb137831ab819326116"
        ),
        format!("run-notification:final:{run_id}"),
    ];
    expected.sort();
    assert_eq!(
        projection_refs, expected,
        "the triggered caller must pass each canonical gate_ref into the pinned digest contract \
         while preserving the legacy final ID"
    );
}

#[tokio::test]
async fn triggered_claim_loser_records_no_delivered_outcome_marker_or_gate_route() {
    const GATE_REF: &str = "gate:triggered-claim-loser";
    let unconfirmed_states = [
        (OutboundDeliveryStatus::Sending, None),
        (OutboundDeliveryStatus::Unknown, None),
        (OutboundDeliveryStatus::Pending, None),
        (
            OutboundDeliveryStatus::Failed,
            Some(DeliveryFailureKind::Rejected),
        ),
        (
            OutboundDeliveryStatus::DeadLettered,
            Some(DeliveryFailureKind::TransportUnavailable),
        ),
    ];

    for (status, failure_kind) in unconfirmed_states {
        let (harness, run_notification_claim_calls, list_calls) =
            build_claim_loss_triggered_harness(
                scripted_state(TurnStatus::BlockedApproval, Some(GATE_REF)),
                status,
                failure_kind,
            );
        seed_preference(&harness.store).await;
        let run_id = TurnRunId::new();

        harness
            .driver
            .on_trigger_submitted(triggered_request(run_id, false))
            .await;

        assert_eq!(
            wait_for_outcome(&harness.delivery_store, run_id).await,
            TriggeredRunDeliveryOutcomeKind::Failed,
            "a {status:?} existing row must remain non-success"
        );
        assert_eq!(
            list_calls.load(Ordering::SeqCst),
            1,
            "the coordinator must recover the scope once and consume the authoritative claim outcome"
        );
        assert_eq!(run_notification_claim_calls.load(Ordering::SeqCst), 1);
        assert!(
            harness.adapter.envelopes().is_empty(),
            "the losing worker must never drive vendor egress"
        );
        assert!(
            harness
                .route_store
                .load_delivered_gate_route(&tenant(), &user(), GATE_REF)
                .await
                .expect("route lookup")
                .is_none(),
            "unconfirmed delivery must not publish a gate route or advance the blocked marker"
        );
    }
}

#[tokio::test]
async fn triggered_observed_delivered_duplicate_is_the_only_confirmed_claim_loss() {
    const GATE_REF: &str = "gate:triggered-confirmed-duplicate";
    let (harness, _, list_calls) = build_claim_loss_triggered_harness(
        scripted_state(TurnStatus::BlockedApproval, Some(GATE_REF)),
        OutboundDeliveryStatus::Delivered,
        None,
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
        list_calls.load(Ordering::SeqCst),
        1,
        "the coordinator must recover the scope once and consume the authoritative claim outcome"
    );
    assert!(harness.adapter.envelopes().is_empty());
    assert!(
        harness
            .route_store
            .load_delivered_gate_route(&tenant(), &user(), GATE_REF)
            .await
            .expect("route lookup")
            .is_some()
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
    seed_final_message_with_attachments(
        &harness.threads,
        run_id,
        "trigger complete: /workspace/trigger.json",
        vec![AttachmentRef {
            id: "reply-attachment-0".to_string(),
            kind: AttachmentKind::Document,
            mime_type: "application/json".to_string(),
            filename: Some("trigger.json".to_string()),
            size_bytes: Some(11),
            storage_key: Some("/workspace/trigger.json".to_string()),
            extracted_text: None,
        }],
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

#[tokio::test]
async fn triggered_oauth_non_dm_settlement_failure_still_cancels_and_sends_only_safe_replacement() {
    const GATE_REF: &str = "gate:auth-settlement-failure";
    const OAUTH_SECRET: &str = "https://provider.example/oauth?code=must-not-leak";
    let harness = build_settlement_failure_triggered_harness(
        scripted_state(TurnStatus::BlockedAuth, Some(GATE_REF)),
        Some(OAUTH_SECRET),
        false,
        false,
    );
    seed_preference(&harness.store).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    assert_eq!(
        wait_for_outcome(&harness.delivery_store, run_id).await,
        TriggeredRunDeliveryOutcomeKind::Delivered,
        "the safe replacement, not the failed OAuth prompt, owns terminal success"
    );
    assert_eq!(
        harness.turns.cancel_call_count(),
        1,
        "the nested target failure must retain the OAuth non-DM classification"
    );
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 1, "only the safe replacement may reach egress");
    assert!(texts[0].contains("Ironclaw web app"), "{}", texts[0]);
    assert!(!texts[0].contains("Setup link:"), "{}", texts[0]);
    assert!(!texts[0].contains(OAUTH_SECRET), "{}", texts[0]);
    assert!(
        harness
            .route_store
            .load_delivered_gate_route(&tenant(), &user(), GATE_REF)
            .await
            .expect("route lookup")
            .is_none(),
        "the OAuth prompt was never confirmed delivered"
    );
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("delivery attempts");
    assert_eq!(
        attempts.len(),
        2,
        "failed OAuth prompt plus safe replacement"
    );
    assert!(attempts.iter().any(|attempt| {
        attempt
            .candidate
            .projection_ref
            .as_str()
            .starts_with("run-notification:auth:")
            && attempt.status == OutboundDeliveryStatus::Prepared
    }));
    assert!(attempts.iter().any(|attempt| {
        attempt
            .candidate
            .projection_ref
            .as_str()
            .starts_with("run-notification:final:")
            && attempt.status == OutboundDeliveryStatus::Delivered
    }));
    assert!(attempts.iter().all(|attempt| {
        !attempt
            .candidate
            .projection_ref
            .as_str()
            .starts_with("system-notice:cleanup:")
    }));
}

#[tokio::test]
async fn other_combined_settlement_errors_remain_generic_non_success_without_side_effects() {
    const GATE_REF: &str = "gate:approval-settlement-failure";
    let harness = build_settlement_failure_triggered_harness(
        scripted_state(TurnStatus::BlockedApproval, Some(GATE_REF)),
        None,
        true,
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
        TriggeredRunDeliveryOutcomeKind::Failed,
        "a non-OAuth combined failure must not be promoted to delivery success"
    );
    assert_eq!(harness.turns.cancel_call_count(), 0);
    assert!(harness.adapter.envelopes().is_empty(), "zero vendor egress");
    assert!(
        harness
            .route_store
            .load_delivered_gate_route(&tenant(), &user(), GATE_REF)
            .await
            .expect("route lookup")
            .is_none(),
        "a failed settlement cannot authorize a gate route"
    );
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("delivery attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, OutboundDeliveryStatus::Prepared);
    assert!(attempts.iter().all(|attempt| {
        !attempt
            .candidate
            .projection_ref
            .as_str()
            .starts_with("system-notice:cleanup:")
    }));
}
