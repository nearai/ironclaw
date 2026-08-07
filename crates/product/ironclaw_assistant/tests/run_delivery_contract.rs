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
use ironclaw_assistant::{
    DeliveryCoordinator, DeliveryRetryPolicy, RunDeliveryObserver, RunDeliveryServices,
    RunDeliverySettings, TriggeredRunDeliveryDriver,
};
use ironclaw_assistant::{
    ProjectFilesystemReader, ProjectFsEntry, ProjectFsEntryKind, ProjectFsError, ProjectFsStat,
};
use ironclaw_extension_contracts::auth_prompt::AuthPromptView;
use ironclaw_extension_contracts::channel_adapter::ChannelAdapter;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelError, DeliveryReport, InboundOutcome, OutboundEnvelope, OutboundPart,
    PartDeliveryOutcome, ProductTriggerReason, VerifiedInbound,
};
use ironclaw_extension_contracts::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId,
};
use ironclaw_extension_contracts::preference_target::{
    PreferenceTargetCodec, PreferenceTargetEncodeRequest,
};
use ironclaw_host_api::product_adapter::auth::{AuthRequirement, ProtocolAuthEvidence};
use ironclaw_host_api::product_adapter::{
    AdapterInstallationId, ProductAdapterError, ProductAdapterId,
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
    CommunicationModality, CommunicationPreferenceKey, CommunicationPreferenceRecord,
    CommunicationPreferenceRepository, DeliveredGateRouteStore, DeliveryDefaultScope,
    DeliveryTargetCapabilities, OutboundError, OutboundStateStore, OutboundStateStorePort,
    TriggeredFireFailureDeliveryRequest, TriggeredRunDeliveryOutcomeKind,
    TriggeredRunDeliveryRequest, TriggeredRunDeliveryStore, VersionedCommunicationPreferenceRecord,
    WriteCommunicationPreferenceRequest,
};
use ironclaw_outbound::{
    OutboundDeliveryTargetEntry, OutboundDeliveryTargetId, OutboundDeliveryTargetOwner,
    OutboundDeliveryTargetProvider, OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary,
};
use ironclaw_product_contracts::account_setup::ChannelConnectionNoticePolicy;
use ironclaw_product_contracts::delivery::{
    ChannelDeliveryResolver, DeliveryReplyContextSource, ResolvedChannelDelivery,
};
use ironclaw_product_contracts::inbound::{
    InboundCommandPayload, ParsedProductInbound, ProductCommandResultPayload, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductRejection, ProductRejectionKind,
    TrustedInboundContext, UserMessagePayload,
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
    binding: ironclaw_product_contracts::binding::ResolvedBinding,
    fail: bool,
}

#[async_trait]
impl ironclaw_product_contracts::binding::ProductBindingResolver for StaticBindingService {
    async fn resolve_binding(
        &self,
        _request: ironclaw_product_contracts::binding::ResolveBindingRequest,
    ) -> Result<
        ironclaw_product_contracts::binding::ResolvedBinding,
        ironclaw_product_contracts::error::ProductOperationFailure,
    > {
        if self.fail {
            return Err(
                ironclaw_product_contracts::error::ProductOperationFailure::BindingResolutionFailed {
                    reason: "unbound".to_string(),
                },
            );
        }
        Ok(self.binding.clone())
    }

    async fn lookup_binding(
        &self,
        _request: ironclaw_product_contracts::binding::ResolveBindingRequest,
    ) -> Result<
        ironclaw_product_contracts::binding::ResolvedBinding,
        ironclaw_product_contracts::error::ProductOperationFailure,
    > {
        if self.fail {
            return Err(
                ironclaw_product_contracts::error::ProductOperationFailure::BindingResolutionFailed {
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

/// One entry in the scripted notification catalog: an opaque catalog id, the
/// vendor binding ref it resolves to, the conversation that ref decodes back
/// to, and whether the catalog entry is a personal DM.
#[derive(Clone, Copy)]
struct TestNotificationTarget {
    target_id: &'static str,
    /// The channel extension that owns this target's binding grammar — the
    /// notifier reads it off the catalog entry to pick a delivering channel.
    extension_id: &'static str,
    binding_ref: &'static str,
    conversation_id: &'static str,
    direct_message: bool,
}

/// The creator's personal DM — the only class of target an OAuth
/// `authorization_url` may land in.
const DM_TARGET: TestNotificationTarget = TestNotificationTarget {
    target_id: "acme:personal-dm:user-a",
    extension_id: EXTENSION_ID,
    binding_ref: "reply:acme:dm",
    conversation_id: "dm-creator",
    direct_message: true,
};

/// A shared channel the creator picked as a notification channel.
const SHARED_TARGET: TestNotificationTarget = TestNotificationTarget {
    target_id: "acme:shared-channel:eng",
    extension_id: EXTENSION_ID,
    binding_ref: "reply:acme:eng",
    conversation_id: "chan-eng",
    direct_message: false,
};

/// A second channel extension's personal DM, activated only PARTWAY through
/// `triggered_gate_prompt_reaches_a_channel_activated_after_the_first_fire`.
const LATE_EXTENSION_ID: &str = "beta";
const LATE_ACTIVATED_TARGET: TestNotificationTarget = TestNotificationTarget {
    target_id: "beta:personal-dm:user-a",
    extension_id: LATE_EXTENSION_ID,
    binding_ref: "reply:beta:dm",
    conversation_id: "dm-beta",
    direct_message: true,
};

/// Codec over the scripted catalog: decodes exactly the binding refs the
/// catalog minted, and answers the DM predicate per target (a single-bool
/// codec cannot express a mixed DM/non-DM notification set).
struct CatalogCodec {
    targets: Vec<TestNotificationTarget>,
}

impl CatalogCodec {
    fn find(&self, target: &ReplyTargetBindingRef) -> Option<&TestNotificationTarget> {
        self.targets
            .iter()
            .find(|entry| entry.binding_ref == target.as_str())
    }
}

impl PreferenceTargetCodec for CatalogCodec {
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef> {
        let entry = self.find(target)?;
        ExternalConversationRef::new(Some("space-1"), entry.conversation_id, None, None).ok()
    }

    fn is_personal_direct_message(&self, target: &ReplyTargetBindingRef) -> bool {
        self.find(target).is_some_and(|entry| entry.direct_message)
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

/// A LIVE codec source whose active set can grow mid-test — the seam that
/// distinguishes "codecs re-read per fire" from "codecs frozen at build".
#[derive(Default)]
struct GrowableCodecs {
    codecs: Mutex<Vec<Arc<dyn PreferenceTargetCodec>>>,
}

impl GrowableCodecs {
    fn with_initial(targets: Vec<TestNotificationTarget>) -> Self {
        Self {
            codecs: Mutex::new(vec![
                Arc::new(CatalogCodec { targets }) as Arc<dyn PreferenceTargetCodec>
            ]),
        }
    }

    /// Stand-in for a channel extension activating: its codec joins the
    /// active set.
    fn activate(&self, targets: Vec<TestNotificationTarget>) {
        self.codecs
            .lock()
            .expect("codecs")
            .push(Arc::new(CatalogCodec { targets }) as Arc<dyn PreferenceTargetCodec>);
    }
}

impl ironclaw_extension_contracts::preference_target::ActivePreferenceTargetCodecs
    for GrowableCodecs
{
    fn active_preference_target_codecs(&self) -> Vec<Arc<dyn PreferenceTargetCodec>> {
        self.codecs.lock().expect("codecs").clone()
    }
}

/// The owner-scoped catalog the background-run notifier resolves stored
/// notification-target ids through. Every entry is owned by the calling
/// scope and carries the `acme` extension id in its `channel` field, which is
/// where the notifier reads the delivering extension from.
struct StaticTargetCatalog {
    targets: Vec<TestNotificationTarget>,
}

#[async_trait]
impl OutboundDeliveryTargetProvider for StaticTargetCatalog {
    async fn list_outbound_delivery_targets(
        &self,
        scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        Ok(self
            .targets
            .iter()
            .map(|entry| OutboundDeliveryTargetEntry {
                summary: OutboundDeliveryTargetSummary::new(
                    OutboundDeliveryTargetId::new(entry.target_id).expect("target id"),
                    entry.extension_id,
                    entry.conversation_id,
                    None,
                )
                .expect("target summary"),
                capabilities: DeliveryTargetCapabilities {
                    final_replies: true,
                    progress: false,
                    gate_prompts: true,
                    auth_prompts: true,
                    modalities: Vec::new(),
                },
                destination: ReplyTargetBindingRef::new(entry.binding_ref).expect("binding ref"),
                owner: OutboundDeliveryTargetOwner::for_scope(scope),
            })
            .collect())
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

fn binding() -> ironclaw_product_contracts::binding::ResolvedBinding {
    ironclaw_product_contracts::binding::ResolvedBinding {
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
    project_files: Arc<ScriptedProjectFilesystemReader>,
    connection_notices: ChannelConnectionNoticePolicy,
    adapter: Arc<RecordingChannelAdapter>,
    store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    route_store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    turns: Arc<ScriptedTurnCoordinator>,
    threads: Arc<InMemorySessionThreadService>,
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
        delivery_targets: Arc::new(StaticTargetCatalog {
            targets: Vec::new(),
        }) as Arc<dyn OutboundDeliveryTargetProvider>,
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
        project_files,
        connection_notices,
        adapter,
        store,
        route_store,
        turns,
        threads,
    }
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
            ironclaw_product_contracts::inbound::ApprovalResolutionPayload::new(
                "gate-1",
                ironclaw_product_contracts::inbound::ApprovalDecision::ApproveOnce,
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

fn triggered_request(run_id: TurnRunId, project_scoped: bool) -> TriggeredRunDeliveryRequest {
    TriggeredRunDeliveryRequest {
        run_id,
        scope: binding_scope(),
        creator_user_id: user(),
        project_scoped,
        prompt: "watch the deploys".to_string(),
    }
}

struct TriggeredHarness {
    driver: TriggeredRunDeliveryDriver,
    codecs: Arc<GrowableCodecs>,
    adapter: Arc<RecordingChannelAdapter>,
    store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    route_store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    delivery_store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    turns: Arc<ScriptedTurnCoordinator>,
    threads: Arc<InMemorySessionThreadService>,
}

/// `catalog` is the creator-owned notification catalog the notifier resolves
/// stored target ids through; the same entries back the codec, so a resolved
/// id decodes to a conversation and answers the DM predicate consistently.
fn build_triggered_harness(
    states: Vec<ScriptedRunState>,
    auth_url: Option<&str>,
    catalog: Vec<TestNotificationTarget>,
) -> TriggeredHarness {
    let initially_active = catalog.clone();
    build_triggered_harness_with_initial_codecs(states, auth_url, catalog, initially_active)
}

/// [`build_triggered_harness`] with the ACTIVE codec set narrower than the
/// catalog, so a test can activate the rest partway through.
fn build_triggered_harness_with_initial_codecs(
    states: Vec<ScriptedRunState>,
    auth_url: Option<&str>,
    catalog: Vec<TestNotificationTarget>,
    initially_active: Vec<TestNotificationTarget>,
) -> TriggeredHarness {
    build_triggered_harness_with_preferences(states, auth_url, catalog, initially_active, None)
}

fn build_triggered_harness_with_preferences(
    states: Vec<ScriptedRunState>,
    auth_url: Option<&str>,
    catalog: Vec<TestNotificationTarget>,
    initially_active: Vec<TestNotificationTarget>,
    communication_preferences: Option<Arc<dyn CommunicationPreferenceRepository>>,
) -> TriggeredHarness {
    build_triggered_harness_with_catalog(
        states,
        auth_url,
        catalog,
        initially_active,
        communication_preferences,
        None,
    )
}

/// [`build_triggered_harness_with_preferences`] with an injectable catalog
/// provider, so a test can make per-id lookups fail.
fn build_triggered_harness_with_catalog(
    states: Vec<ScriptedRunState>,
    auth_url: Option<&str>,
    catalog: Vec<TestNotificationTarget>,
    initially_active: Vec<TestNotificationTarget>,
    communication_preferences: Option<Arc<dyn CommunicationPreferenceRepository>>,
    delivery_targets: Option<Arc<dyn OutboundDeliveryTargetProvider>>,
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
    let codecs = Arc::new(GrowableCodecs::with_initial(initially_active));
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
        communication_preferences: communication_preferences
            .unwrap_or_else(|| Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>),
        project_filesystem: Arc::clone(&project_files) as Arc<dyn ProjectFilesystemReader>,
        delivery_targets: delivery_targets.unwrap_or_else(|| {
            Arc::new(StaticTargetCatalog { targets: catalog })
                as Arc<dyn OutboundDeliveryTargetProvider>
        }),
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
        Arc::clone(&codecs)
            as Arc<
                dyn ironclaw_extension_contracts::preference_target::ActivePreferenceTargetCodecs,
            >,
        agent(),
    );
    TriggeredHarness {
        driver,
        codecs,
        adapter,
        store,
        route_store,
        delivery_store,
        turns,
        threads,
    }
}

struct LoadFailingCommunicationPreferences;

#[async_trait]
impl CommunicationPreferenceRepository for LoadFailingCommunicationPreferences {
    async fn load_communication_preference(
        &self,
        _key: CommunicationPreferenceKey,
    ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError> {
        Err(OutboundError::Backend)
    }

    async fn write_communication_preference(
        &self,
        _request: WriteCommunicationPreferenceRequest,
    ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
        Err(OutboundError::Backend)
    }
}

/// Seed the creator's explicit notification-channel set (spec §7's new shape).
async fn seed_notification_targets(
    store: &OutboundStateStore<ironclaw_filesystem::InMemoryBackend>,
    targets: &[TestNotificationTarget],
) {
    store
        .put_communication_preference(CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::personal(tenant(), user()),
            legacy_notification_target: None,
            default_modality: Some(CommunicationModality::Text),
            notification_targets: targets
                .iter()
                .map(|entry| OutboundDeliveryTargetId::new(entry.target_id).expect("target id"))
                .collect(),
            updated_at: Utc::now(),
            updated_by: user(),
        })
        .await
        .expect("preference");
}

/// Seed a pre-notification-set record: only the legacy single slot, which
/// reads back as a one-element notification set.
async fn seed_legacy_single_slot_preference(
    store: &OutboundStateStore<ironclaw_filesystem::InMemoryBackend>,
    target: &TestNotificationTarget,
) {
    store
        .put_communication_preference(CommunicationPreferenceRecord {
            scope: DeliveryDefaultScope::personal(tenant(), user()),
            legacy_notification_target: Some(
                ReplyTargetBindingRef::new(target.binding_ref).expect("binding ref"),
            ),
            default_modality: Some(CommunicationModality::Text),
            notification_targets: Vec::new(),
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

/// Conversations, in delivery order, every notification landed in.
fn delivered_conversations(adapter: &RecordingChannelAdapter) -> Vec<String> {
    adapter
        .envelopes()
        .iter()
        .map(|envelope| envelope.target.conversation.conversation_id().to_string())
        .collect()
}

#[tokio::test]
async fn triggered_project_scoped_fire_is_denied_without_delivery() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        vec![DM_TARGET],
    );
    seed_notification_targets(&harness.store, &[DM_TARGET]).await;
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
async fn permanent_pre_submit_failure_notifies_every_configured_channel_with_no_run() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        vec![DM_TARGET, SHARED_TARGET],
    );
    seed_notification_targets(&harness.store, &[DM_TARGET, SHARED_TARGET]).await;
    let scope = binding_scope();

    let request = TriggeredFireFailureDeliveryRequest {
        scope: scope.clone(),
        creator_user_id: user(),
        project_scoped: false,
        prompt: "Send the daily summary".to_string(),
        failure_ref: ironclaw_outbound::ProjectionUpdateRef::new("trigger-failure:fire-identity-1")
            .expect("failure ref"),
    };
    harness
        .driver
        .on_trigger_failed_before_submit(request.clone())
        .await;

    for _ in 0..100 {
        if harness.adapter.texts().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 2, "one redacted failure notice per target");
    assert!(texts.iter().all(|text| text.contains("routine run failed")));
    assert!(
        texts.iter().all(|text| !text.contains("materialization")),
        "notification copy must not leak internal failure detail: {texts:?}"
    );
    let attempts = harness
        .store
        .list_delivery_attempts(scope)
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 2, "durable attempt evidence per target");
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.candidate.turn_run_id.is_none())
    );

    harness
        .driver
        .on_trigger_failed_before_submit(request)
        .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        harness.adapter.texts().len(),
        2,
        "replaying one settled fire must not duplicate provider sends"
    );
    assert_eq!(
        harness
            .store
            .list_delivery_attempts(binding_scope())
            .await
            .expect("attempts after replay")
            .len(),
        2,
        "stable fire identity must reuse the durable attempts"
    );
}

#[tokio::test]
async fn project_scoped_pre_submit_failure_is_not_sent_to_personal_channels() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        vec![DM_TARGET],
    );
    seed_notification_targets(&harness.store, &[DM_TARGET]).await;
    let scope = binding_scope();

    harness
        .driver
        .on_trigger_failed_before_submit(TriggeredFireFailureDeliveryRequest {
            scope: scope.clone(),
            creator_user_id: user(),
            project_scoped: true,
            prompt: "Prepare the project report".to_string(),
            failure_ref: ironclaw_outbound::ProjectionUpdateRef::new(
                "trigger-failure:project-fire",
            )
            .expect("failure ref"),
        })
        .await;

    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(harness.adapter.texts().is_empty());
    assert!(
        harness
            .store
            .list_delivery_attempts(scope)
            .await
            .expect("attempts")
            .is_empty()
    );
}

/// Spec §8: the result push is gone. A background run that finishes normally
/// records its answer in the fire's own run thread and puts NOTHING on a
/// channel — delivery is the model's explicit `builtin.outbound_deliver`
/// call, never an automatic push.
#[tokio::test]
async fn triggered_completed_run_delivers_nothing_external() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        vec![DM_TARGET, SHARED_TARGET],
    );
    // Preferences live on the outbound store; seed the creator's notification
    // set and pin that the notifier resolves from the SAME store handle.
    assert!(Arc::ptr_eq(
        &(Arc::clone(&harness.store) as Arc<dyn CommunicationPreferenceRepository>),
        &harness.driver.communication_preferences()
    ));
    seed_notification_targets(&harness.store, &[DM_TARGET, SHARED_TARGET]).await;
    let run_id = TurnRunId::new();
    seed_final_message(&harness.threads, run_id, "deploy watch complete").await;

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Skipped);
    assert!(
        harness.adapter.texts().is_empty(),
        "a completed background run must not push its result to a channel: {:?}",
        harness.adapter.texts()
    );
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert!(attempts.is_empty(), "no delivery attempt: {attempts:?}");
}

/// Spec §7: an approval gate raised by a background run reaches EVERY
/// configured notification channel, one coordinated delivery each, so the
/// creator can approve from whichever surface they see first.
#[tokio::test]
async fn triggered_gate_prompt_fans_out_to_every_notification_target() {
    let harness = build_triggered_harness(
        vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some("gate:approval-00000000000000000000000000000009"),
        )],
        None,
        vec![DM_TARGET, SHARED_TARGET],
    );
    seed_notification_targets(&harness.store, &[DM_TARGET, SHARED_TARGET]).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 2, "one gate prompt per target: {texts:?}");
    assert!(
        texts.iter().all(|text| text.contains("Approval needed")),
        "{texts:?}"
    );
    assert_eq!(
        delivered_conversations(&harness.adapter),
        vec!["dm-creator".to_string(), "chan-eng".to_string()],
        "fan-out follows the stored notification-channel order"
    );
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert_eq!(attempts.len(), 2, "one persisted attempt per target");
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.status == ironclaw_outbound::OutboundDeliveryStatus::Delivered)
    );
    // Every conversation the prompt landed in can carry a bare `approve`
    // reply back to this gate.
    let route = harness
        .route_store
        .load_delivered_gate_route(
            &tenant(),
            &user(),
            "gate:approval-00000000000000000000000000000009",
        )
        .await
        .expect("route lookup")
        .expect("gate route recorded");
    assert_eq!(route.run_id, run_id);
    for conversation_id in ["dm-creator", "chan-eng"] {
        let fingerprint = ironclaw_extension_contracts::external::ExternalConversationRef::new(
            Some("space-1"),
            conversation_id,
            None,
            None,
        )
        .expect("conversation")
        .conversation_fingerprint();
        assert!(
            route
                .delivered_conversation_fingerprints
                .contains(&fingerprint),
            "gate route must cover {conversation_id}: {:?}",
            route.delivered_conversation_fingerprints
        );
    }
}

/// Spec §7: an OAuth `authorization_url` may only land in a personal DM.
/// Non-DM notification channels get a redacted "needs re-auth, open the app"
/// notice instead, and the run is NO LONGER cancelled — it parks so the user
/// can finish the re-auth and let the routine resume.
#[tokio::test]
async fn triggered_auth_prompt_reaches_only_dm_targets_and_run_stays_parked() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::BlockedAuth, Some("gate:auth-t"))],
        Some("https://provider.example/oauth"),
        vec![DM_TARGET, SHARED_TARGET],
    );
    seed_notification_targets(&harness.store, &[DM_TARGET, SHARED_TARGET]).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(
        outcome,
        TriggeredRunDeliveryOutcomeKind::Delivered,
        "the run parks awaiting the user after its prompt went out"
    );
    assert_eq!(
        harness.turns.cancel_call_count(),
        0,
        "an OAuth-blocked background run parks; it is never cancelled for lack of a DM target"
    );
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 2, "one message per target: {texts:?}");
    assert!(
        texts[0].contains("Setup link: https://provider.example/oauth"),
        "the personal DM carries the authorization URL: {}",
        texts[0]
    );
    assert!(
        !texts[1].contains("Setup link:"),
        "a shared channel must never carry the authorization URL: {}",
        texts[1]
    );
    assert!(
        texts[1].contains("A routine needs re-authorization"),
        "the shared channel gets the redacted notice: {}",
        texts[1]
    );
    assert_eq!(
        delivered_conversations(&harness.adapter),
        vec!["dm-creator".to_string(), "chan-eng".to_string()]
    );
}

/// Spec §7: manual-token (non-OAuth) auth keeps today's cancel behavior — a
/// secret can never be typed into a chat — and the notice fans out to every
/// notification channel.
#[tokio::test]
async fn triggered_manual_token_auth_cancels_and_notifies_all_targets() {
    // No auth-prompt source wired -> non-OAuth challenge.
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::BlockedAuth, Some("gate:auth-m"))],
        None,
        vec![DM_TARGET, SHARED_TARGET],
    );
    seed_notification_targets(&harness.store, &[DM_TARGET, SHARED_TARGET]).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    assert_eq!(harness.turns.cancel_call_count(), 1, "run cancelled");
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 2, "one notice per target: {texts:?}");
    assert!(
        texts.iter().all(|text| text.contains("Ironclaw web app")),
        "{texts:?}"
    );
    assert_eq!(
        delivered_conversations(&harness.adapter),
        vec!["dm-creator".to_string(), "chan-eng".to_string()]
    );
}

/// Spec §7: a failed background run tells every notification channel, so a
/// silent routine failure is never the user's first sign something broke.
#[tokio::test]
async fn triggered_failure_notifies_all_targets() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Failed, None)],
        None,
        vec![DM_TARGET, SHARED_TARGET],
    );
    seed_notification_targets(&harness.store, &[DM_TARGET, SHARED_TARGET]).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    let texts = harness.adapter.texts();
    assert_eq!(texts.len(), 2, "one failure notice per target: {texts:?}");
    assert!(
        texts.iter().all(|text| text.contains("routine run failed")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .all(|text| text.contains("From a triggered event: “watch the deploys”.")),
        "the failure notice names the routine: {texts:?}"
    );
}

/// Spec §7: with no notification channels configured, notifications live in
/// the web app only. The blocked run is untouched (no cancel, no resume) and
/// NOTHING is attempted externally.
#[tokio::test]
async fn triggered_empty_notification_set_delivers_nothing() {
    let harness = build_triggered_harness(
        vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some("gate:approval-00000000000000000000000000000010"),
        )],
        None,
        vec![DM_TARGET, SHARED_TARGET],
    );
    // Catalog entries exist, but the creator selected none of them.
    seed_notification_targets(&harness.store, &[]).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(
        outcome,
        TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured
    );
    assert!(harness.adapter.texts().is_empty(), "nothing delivered");
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert!(attempts.is_empty(), "no delivery attempt: {attempts:?}");
}

#[tokio::test]
async fn triggered_notification_preference_read_failure_is_not_reported_as_no_configuration() {
    let harness = build_triggered_harness_with_preferences(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        vec![DM_TARGET],
        vec![DM_TARGET],
        Some(Arc::new(LoadFailingCommunicationPreferences)),
    );
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    assert_eq!(
        wait_for_outcome(&harness.delivery_store, run_id).await,
        TriggeredRunDeliveryOutcomeKind::Failed,
        "a storage outage must stay distinguishable from an intentionally empty channel set"
    );
    assert!(harness.adapter.texts().is_empty(), "nothing was resolved");
}

/// Regression: the notifier reads the ACTIVE codec set at every fire.
///
/// The notifier is built once and lives for the process. When it captured
/// `active_preference_codecs()` at construction, a channel extension activated
/// afterwards could never decode its own binding refs: its notification
/// targets classified as non-DM and then failed metadata resolution, so that
/// channel's gate prompts silently never arrived until a restart (warn-only).
/// Both fires here run through the SAME notifier; only the active codec set
/// changes between them.
#[tokio::test]
async fn triggered_gate_prompt_reaches_a_channel_activated_after_the_first_fire() {
    let harness = build_triggered_harness_with_initial_codecs(
        vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some("gate:approval-00000000000000000000000000000012"),
        )],
        None,
        // The creator picked both channels; the catalog resolves both.
        vec![DM_TARGET, LATE_ACTIVATED_TARGET],
        // ...but only the first extension is active when the notifier is built.
        vec![DM_TARGET],
    );
    seed_notification_targets(&harness.store, &[DM_TARGET, LATE_ACTIVATED_TARGET]).await;

    let first_run = TurnRunId::new();
    harness
        .driver
        .on_trigger_submitted(triggered_request(first_run, false))
        .await;
    assert_eq!(
        wait_for_outcome(&harness.delivery_store, first_run).await,
        TriggeredRunDeliveryOutcomeKind::Delivered
    );
    assert_eq!(
        delivered_conversations(&harness.adapter),
        vec!["dm-creator".to_string()],
        "only the active channel can be decoded on the first fire"
    );

    // The second channel extension activates.
    harness.codecs.activate(vec![LATE_ACTIVATED_TARGET]);

    let second_run = TurnRunId::new();
    harness
        .driver
        .on_trigger_submitted(triggered_request(second_run, false))
        .await;
    assert_eq!(
        wait_for_outcome(&harness.delivery_store, second_run).await,
        TriggeredRunDeliveryOutcomeKind::Delivered
    );
    assert_eq!(
        delivered_conversations(&harness.adapter),
        vec![
            "dm-creator".to_string(),
            "dm-creator".to_string(),
            "dm-beta".to_string()
        ],
        "the second fire reaches the newly activated channel too"
    );
    // The late channel's delivery went out through ITS OWN extension, read
    // from the catalog entry — not the notifier's attribution bucket.
    let envelopes = harness.adapter.envelopes();
    let late = envelopes
        .iter()
        .find(|envelope| envelope.target.conversation.conversation_id() == "dm-beta")
        .expect("the late channel received a gate prompt");
    assert_eq!(late.extension_id, LATE_EXTENSION_ID);
}

/// The discriminating half of the empty-set rule.
///
/// Manual-token auth is the ONE arm with a run-mutating side effect: with a
/// notification channel present it cancels the run, because a credential can
/// never be typed into a chat (see
/// `triggered_manual_token_auth_cancels_and_notifies_all_targets`). With NO
/// channel there is no chat either — the user completes the credential in the
/// web app — so the notifier must leave the run alone. A `BlockedApproval`
/// arm cannot prove this: that arm never cancels under any configuration.
#[tokio::test]
async fn triggered_empty_notification_set_leaves_a_manual_token_auth_run_parked() {
    // No auth-prompt source wired -> the non-OAuth (manual credential) arm.
    let harness = build_triggered_harness(
        vec![scripted_state(
            TurnStatus::BlockedAuth,
            Some("gate:auth-empty"),
        )],
        None,
        vec![DM_TARGET, SHARED_TARGET],
    );
    // Catalog entries exist, but the creator selected none of them.
    seed_notification_targets(&harness.store, &[]).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    // The load-bearing assertion: the notifier must not have run the
    // manual-token arm's cancel side effect at all.
    assert_eq!(
        harness.turns.cancel_call_count(),
        0,
        "with no notification channel the run is left parked for the web app, never cancelled"
    );
    assert_eq!(
        outcome,
        TriggeredRunDeliveryOutcomeKind::NoDefaultConfigured
    );
    assert!(harness.adapter.texts().is_empty(), "nothing delivered");
    let attempts = harness
        .store
        .list_delivery_attempts(binding_scope())
        .await
        .expect("attempts");
    assert!(attempts.is_empty(), "no delivery attempt: {attempts:?}");
}

/// Spec §7 read-time migration: a record written before notification sets
/// existed carries only the legacy single slot. It reads back as a one-element
/// notification set, so today's users keep getting their notifications.
#[tokio::test]
async fn triggered_legacy_single_slot_preference_notifies_that_target() {
    let harness = build_triggered_harness(
        vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some("gate:approval-00000000000000000000000000000011"),
        )],
        None,
        vec![DM_TARGET, SHARED_TARGET],
    );
    seed_legacy_single_slot_preference(&harness.store, &SHARED_TARGET).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    assert_eq!(
        delivered_conversations(&harness.adapter),
        vec!["chan-eng".to_string()],
        "only the migrated legacy slot is notified"
    );
}

#[tokio::test]
async fn triggered_delivery_is_skipped_when_the_pending_queue_is_full() {
    let harness = build_triggered_harness(
        vec![scripted_state(TurnStatus::Completed, None)],
        None,
        vec![DM_TARGET],
    );
    seed_notification_targets(&harness.store, &[DM_TARGET]).await;
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

/// A catalog whose per-id resolution always fails — a backend outage at fire
/// time, as distinct from a target that resolves cleanly to "not yours".
struct FailingTargetCatalog;

#[async_trait]
impl OutboundDeliveryTargetProvider for FailingTargetCatalog {
    async fn list_outbound_delivery_targets(
        &self,
        _scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        Err(OutboundError::Backend)
    }

    async fn resolve_outbound_delivery_target(
        &self,
        _scope: &OutboundDeliveryTargetScope,
        _target_id: &OutboundDeliveryTargetId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
        Err(OutboundError::Backend)
    }
}

/// An outage that eats every configured channel must NOT be recorded as the
/// benign "user configured nothing" state.
///
/// The notifier skips per-id lookup failures so one unreachable channel cannot
/// suppress the rest — but when every lookup fails the target list is empty for
/// a completely different reason, and recording `NoDefaultConfigured` there
/// makes a backend outage durably indistinguishable from an opt-out. The
/// sibling preference-read failure already records `Failed`; this is the same
/// honesty one layer down.
#[tokio::test]
async fn triggered_all_catalog_lookups_failing_is_not_reported_as_no_configuration() {
    let harness = build_triggered_harness_with_catalog(
        vec![scripted_state(
            TurnStatus::BlockedApproval,
            Some("gate:approval-00000000000000000000000000000010"),
        )],
        None,
        vec![DM_TARGET, SHARED_TARGET],
        vec![DM_TARGET, SHARED_TARGET],
        None,
        Some(Arc::new(FailingTargetCatalog) as Arc<dyn OutboundDeliveryTargetProvider>),
    );
    // The creator DID configure channels; the catalog cannot resolve them.
    seed_notification_targets(&harness.store, &[DM_TARGET]).await;
    let run_id = TurnRunId::new();

    harness
        .driver
        .on_trigger_submitted(triggered_request(run_id, false))
        .await;

    let outcome = wait_for_outcome(&harness.delivery_store, run_id).await;
    assert_eq!(
        outcome,
        TriggeredRunDeliveryOutcomeKind::Failed,
        "an outage that resolved no channels must not read as an intentional empty set"
    );
    assert!(
        harness.adapter.texts().is_empty(),
        "nothing is delivered when no channel resolves"
    );
}
