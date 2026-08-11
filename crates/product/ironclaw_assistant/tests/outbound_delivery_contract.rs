// arch-exempt: large_file, mechanical OutboundStateStore<ironclaw_filesystem::InMemoryBackend> -> OutboundStateStore<InMemoryBackend> §4.3 store consolidation, no logic change, plan #6168
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_assistant::{
    ProductOutboundTargetResolver, ProductSurfaceFailure, ProjectFilesystemReader, ProjectFsEntry,
    ProjectFsEntryKind, ProjectFsError, ProjectFsStat, VerifiedProductOutboundTargetMetadata,
};
use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_extension_contracts::external::{ExternalActorRef, ExternalConversationRef};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{
    attachment::WorkspaceFile,
    ids::{AgentId, ExtensionId, ProjectId, TenantId, ThreadId, UserId},
    path::ScopedPath,
    product_adapter::AdapterInstallationId,
};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    CommunicationPreferenceKey, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    CommunicationPreferenceVersion, DeliveryDefaultScope, OutboundDeliveryAttempt, OutboundError,
    OutboundPolicyService, OutboundStateStore, OutboundStateStorePort, ReplyTargetBindingClaim,
    ReplyTargetBindingValidator, RunNotificationContext, RunNotificationEventKind,
    RunNotificationOrigin, ThreadProjectionAccessClaim, ThreadProjectionAccessPolicy,
    ThreadProjectionAccessRequest, VersionedCommunicationPreferenceRecord,
    WriteCommunicationPreferenceRequest,
};
use ironclaw_threads::{AttachmentKind, AttachmentRef, ThreadScope};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};

#[derive(Default)]
struct AllowAllProjectionAccessPolicy;

static ACCESS_POLICY: AllowAllProjectionAccessPolicy = AllowAllProjectionAccessPolicy;

#[async_trait]
impl ThreadProjectionAccessPolicy for AllowAllProjectionAccessPolicy {
    async fn authorize_projection_access(
        &self,
        request: ThreadProjectionAccessRequest,
    ) -> Result<ThreadProjectionAccessClaim, OutboundError> {
        Ok(ThreadProjectionAccessClaim {
            actor: request.actor,
            scope: request.scope,
            thread_id: request.thread_id,
        })
    }
}

#[derive(Default)]
struct FakeReplyTargetBindingValidator {
    allowed_targets: Mutex<HashSet<ReplyTargetBindingRef>>,
}

impl FakeReplyTargetBindingValidator {
    fn allow(&self, target: ReplyTargetBindingRef) {
        self.allowed_targets
            .lock()
            .expect("validator lock")
            .insert(target);
    }
}

#[async_trait]
impl ReplyTargetBindingValidator for FakeReplyTargetBindingValidator {
    async fn validate_reply_target(
        &self,
        request: ironclaw_outbound::ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        let allowed_targets = self.allowed_targets.lock().expect("validator lock");
        if allowed_targets.contains(&request.candidate.target) {
            Ok(ReplyTargetBindingClaim::new(request.candidate.target))
        } else {
            Err(OutboundError::AccessDenied)
        }
    }
}

#[derive(Default)]
struct FakePreferenceRepository {
    records: Mutex<HashMap<CommunicationPreferenceKey, VersionedCommunicationPreferenceRecord>>,
}

impl FakePreferenceRepository {
    fn put_record(&self, record: CommunicationPreferenceRecord) {
        self.records.lock().expect("preference lock").insert(
            record.key(),
            VersionedCommunicationPreferenceRecord {
                record,
                version: CommunicationPreferenceVersion::from_raw(1),
            },
        );
    }
}

#[async_trait]
impl CommunicationPreferenceRepository for FakePreferenceRepository {
    async fn put_communication_preference(
        &self,
        record: CommunicationPreferenceRecord,
    ) -> Result<(), OutboundError> {
        self.put_record(record);
        Ok(())
    }

    async fn load_communication_preference(
        &self,
        key: CommunicationPreferenceKey,
    ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError> {
        Ok(self
            .records
            .lock()
            .expect("preference lock")
            .get(&key)
            .cloned())
    }

    async fn write_communication_preference(
        &self,
        request: WriteCommunicationPreferenceRequest,
    ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
        let mut records = self.records.lock().expect("preference lock");
        let key = request.record.key();
        let next_version = match (records.get(&key), request.expected_version) {
            (None, None) => CommunicationPreferenceVersion::from_raw(1),
            (Some(existing), Some(expected)) if existing.version == expected => expected.next(),
            _ => return Err(OutboundError::CasConflict),
        };
        let record = VersionedCommunicationPreferenceRecord {
            record: request.record,
            version: next_version,
        };
        records.insert(key, record.clone());
        Ok(record)
    }
}

struct FakeProductOutboundTargetResolver;

#[async_trait]
impl ProductOutboundTargetResolver for FakeProductOutboundTargetResolver {
    async fn resolve_product_outbound_target_metadata(
        &self,
        _target: &ironclaw_outbound::ValidatedReplyTargetBinding,
        _require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductSurfaceFailure> {
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: ExternalConversationRef::new(
                None,
                "tg-chat-123",
                Some("topic-7"),
                Some("msg-42"),
            )
            .expect("valid external conversation"),
            external_actor_ref: Some(
                ExternalActorRef::new("telegram_user", "777", Some("Telegram user"))
                    .expect("valid external actor"),
            ),
        })
    }
}

fn scope() -> TurnScope {
    TurnScope::new_with_owner(
        TenantId::new("tenant-product-outbound").expect("valid tenant"),
        Some(AgentId::new("agent-product-outbound").expect("valid agent")),
        Some(ProjectId::new("project-product-outbound").expect("valid project")),
        ThreadId::new("thread-product-outbound").expect("valid thread"),
        Some(UserId::new("user-product-outbound").expect("valid user")),
    )
}

fn project_thread_scope() -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new("tenant-product-outbound").expect("valid tenant"),
        agent_id: AgentId::new("agent-product-outbound").expect("valid agent"),
        project_id: Some(ProjectId::new("project-product-outbound").expect("valid project")),
        owner_user_id: Some(UserId::new("user-product-outbound").expect("valid user")),
        mission_id: None,
    }
}

fn actor() -> TurnActor {
    TurnActor::new(UserId::new("user-product-outbound").expect("valid user"))
}

fn validated_reply_target() -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new("tg:-100:_:42").expect("valid telegram reply target")
}

fn delivery_request(scope: TurnScope) -> ironclaw_outbound::PrepareCommunicationDeliveryRequest {
    ironclaw_outbound::PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope,
            actor: actor(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: RunNotificationEventKind::FinalReplyReady,
                origin: RunNotificationOrigin::RunScopedTarget {
                    target: validated_reply_target(),
                },
            }),
        },
        turn_run_id: Some(TurnRunId::new()),
        projection_ref: ironclaw_outbound::ProjectionUpdateRef::new("projection:update:1")
            .expect("valid projection ref"),
        attempted_at: Utc::now(),
    }
}

fn configured_policy<'a>(
    store: &'a OutboundStateStore<InMemoryBackend>,
    validator: &'a FakeReplyTargetBindingValidator,
) -> OutboundPolicyService<'a> {
    OutboundPolicyService::new(store, &ACCESS_POLICY, validator)
}

fn seed_preference(repo: &FakePreferenceRepository, scope: &TurnScope) {
    repo.put_record(preference_record(scope));
}

fn preference_record(scope: &TurnScope) -> CommunicationPreferenceRecord {
    CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(scope.tenant_id.clone(), actor().user_id.clone()),
        legacy_notification_target: Some(validated_reply_target()),
        default_modality: Some(CommunicationModality::Text),
        notification_targets: Vec::new(),
        updated_at: Utc::now(),
        updated_by: UserId::new("pref-updater").expect("valid updater"),
    }
}

// ---------------------------------------------------------------------------
// Delivery coordinator (extension-runtime §5.4; OUT-1..7, ING-11)
// ---------------------------------------------------------------------------

use std::collections::VecDeque;
use std::sync::Arc;

use ironclaw_assistant::{
    CoordinatedDeliveryError, CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest,
    DeliveryCoordinator, DeliveryIntent, DeliveryRetryPolicy, NoticeDeliveryRequest,
};
use ironclaw_extension_contracts::channel_adapter::ChannelAdapter;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelError, DeliveryReport, InboundOutcome, OutboundEnvelope, PartDeliveryOutcome,
    VerifiedInbound,
};
use ironclaw_product_contracts::delivery::{
    ChannelDeliveryResolver, DeliveryReplyContextSource, ResolvedChannelDelivery,
};

struct CoordinatorDenyAllEgress;

#[async_trait]
impl ironclaw_extension_contracts::tool_adapter::RestrictedEgress for CoordinatorDenyAllEgress {
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

/// Scripted channel adapter: pops one report per deliver call, records the
/// envelope it saw, and captures the durable attempt status AT deliver time
/// (proving OUT-3: `Sending` is persisted before any vendor work).
struct ScriptedChannelAdapter {
    reports: Mutex<VecDeque<Result<DeliveryReport, ChannelError>>>,
    envelopes: Mutex<Vec<OutboundEnvelope>>,
    observed_status: Mutex<Vec<ironclaw_outbound::OutboundDeliveryStatus>>,
    store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    scope: TurnScope,
}

impl ScriptedChannelAdapter {
    fn new(
        store: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
        scope: TurnScope,
        reports: Vec<Result<DeliveryReport, ChannelError>>,
    ) -> Self {
        Self {
            reports: Mutex::new(reports.into_iter().collect()),
            envelopes: Mutex::new(Vec::new()),
            observed_status: Mutex::new(Vec::new()),
            store,
            scope,
        }
    }

    fn deliver_calls(&self) -> usize {
        self.envelopes.lock().expect("envelopes lock").len()
    }

    fn envelopes(&self) -> Vec<OutboundEnvelope> {
        self.envelopes.lock().expect("envelopes lock").clone()
    }

    fn observed_statuses(&self) -> Vec<ironclaw_outbound::OutboundDeliveryStatus> {
        self.observed_status.lock().expect("status lock").clone()
    }
}

#[async_trait]
impl ChannelAdapter for ScriptedChannelAdapter {
    fn inbound(&self, _request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        Ok(InboundOutcome::Ignore)
    }

    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        _egress: &dyn ironclaw_extension_contracts::tool_adapter::RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        let attempts = self
            .store
            .list_delivery_attempts(self.scope.clone())
            .await
            .expect("list attempts");
        if let Some(attempt) = attempts.first() {
            self.observed_status
                .lock()
                .expect("status lock")
                .push(attempt.status);
        }
        self.envelopes
            .lock()
            .expect("envelopes lock")
            .push(envelope);
        self.reports
            .lock()
            .expect("reports lock")
            .pop_front()
            .unwrap_or_else(|| Err(ChannelError::Unsupported))
    }
}

struct StaticChannelResolver {
    adapter: Arc<ScriptedChannelAdapter>,
    unavailable: bool,
}

impl ChannelDeliveryResolver for StaticChannelResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        if self.unavailable {
            return None;
        }
        Some(ResolvedChannelDelivery {
            extension_id: ExtensionId::new(extension_id).expect("valid extension id"),
            installation_id: AdapterInstallationId::new("inst-1").expect("valid installation id"),
            adapter: Arc::clone(&self.adapter) as Arc<dyn ChannelAdapter>,
            egress: Arc::new(CoordinatorDenyAllEgress),
        })
    }
}

/// Answers with fixed bytes, and **records the identity it was asked about**.
/// A double that ignores its arguments makes its test vacuous: the assertion
/// that matters at this seam is that the coordinator looks the anchor up under
/// the identity of the channel it *resolved*, not under the extension id the
/// caller asked to deliver to.
#[derive(Default)]
struct FixedReplyContext {
    bytes: Vec<u8>,
    asked: Mutex<Vec<(String, String)>>,
}

impl FixedReplyContext {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            asked: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl DeliveryReplyContextSource for FixedReplyContext {
    async fn reply_context(
        &self,
        extension_id: &ExtensionId,
        installation_id: &AdapterInstallationId,
        _conversation_fingerprint: &str,
    ) -> Option<Vec<u8>> {
        self.asked.lock().expect("lock").push((
            extension_id.as_str().to_string(),
            installation_id.as_str().to_string(),
        ));
        Some(self.bytes.clone())
    }
}

struct OrderedChannelResolver {
    adapter: Arc<ScriptedChannelAdapter>,
    phase: Arc<AtomicU8>,
}

impl ChannelDeliveryResolver for OrderedChannelResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        assert_eq!(self.phase.swap(1, Ordering::SeqCst), 0);
        Some(ResolvedChannelDelivery {
            extension_id: ExtensionId::new(extension_id).expect("valid extension id"),
            installation_id: AdapterInstallationId::new("inst-ordered")
                .expect("valid installation id"),
            adapter: Arc::clone(&self.adapter) as Arc<dyn ChannelAdapter>,
            egress: Arc::new(CoordinatorDenyAllEgress),
        })
    }
}

struct OrderedReplyContext {
    phase: Arc<AtomicU8>,
}

#[async_trait]
impl DeliveryReplyContextSource for OrderedReplyContext {
    async fn reply_context(
        &self,
        _extension_id: &ExtensionId,
        _installation_id: &AdapterInstallationId,
        _conversation_fingerprint: &str,
    ) -> Option<Vec<u8>> {
        assert_eq!(self.phase.swap(2, Ordering::SeqCst), 1);
        None
    }
}

struct OrderedProjectFilesystem {
    phase: Arc<AtomicU8>,
}

#[async_trait]
impl ProjectFilesystemReader for OrderedProjectFilesystem {
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
        if self.phase.swap(4, Ordering::SeqCst) != 3 {
            return Err(ProjectFsError::Internal);
        }
        Ok(WorkspaceFile {
            path: ScopedPath::new(path).expect("scoped path"),
            filename: Some("ordered.pdf".to_string()),
            mime_type: "application/pdf".to_string(),
            bytes: b"ordered".to_vec(),
        })
    }

    async fn stat(
        &self,
        _thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError> {
        if self.phase.swap(3, Ordering::SeqCst) != 2 {
            return Err(ProjectFsError::Internal);
        }
        Ok(ProjectFsStat {
            path: path.to_string(),
            kind: ProjectFsEntryKind::File,
            size_bytes: 7,
            mime_type: "application/pdf".to_string(),
        })
    }
}

static NO_PROJECT_FILESYSTEM: ironclaw_assistant::NoProjectFilesystem =
    ironclaw_assistant::NoProjectFilesystem;

#[derive(Default)]
struct ScriptedProjectFilesystem {
    results: Mutex<HashMap<String, Result<WorkspaceFile, ProjectFsError>>>,
    stats: Mutex<HashMap<String, Result<ProjectFsStat, ProjectFsError>>>,
    reads: Mutex<Vec<String>>,
    stat_calls: Mutex<Vec<String>>,
}

impl ScriptedProjectFilesystem {
    fn insert_file(&self, path: &str, size: usize) {
        self.stats.lock().expect("stats").insert(
            path.to_string(),
            Ok(ProjectFsStat {
                path: path.to_string(),
                kind: ProjectFsEntryKind::File,
                size_bytes: size as u64,
                mime_type: "application/octet-stream".to_string(),
            }),
        );
        self.results.lock().expect("results").insert(
            path.to_string(),
            Ok(WorkspaceFile {
                path: ScopedPath::new(path).expect("scoped path"),
                filename: path.rsplit('/').next().map(str::to_string),
                mime_type: "application/octet-stream".to_string(),
                bytes: vec![b'x'; size],
            }),
        );
    }

    fn insert_error(&self, path: &str, error: ProjectFsError) {
        self.stats
            .lock()
            .expect("stats")
            .insert(path.to_string(), Err(error.clone()));
        self.results
            .lock()
            .expect("results")
            .insert(path.to_string(), Err(error));
    }

    fn read_count(&self) -> usize {
        self.reads.lock().expect("reads").len()
    }

    fn insert_stat_path(&self, requested_path: &str, returned_path: &str, size: u64) {
        self.stats.lock().expect("stats").insert(
            requested_path.to_string(),
            Ok(ProjectFsStat {
                path: returned_path.to_string(),
                kind: ProjectFsEntryKind::File,
                size_bytes: size,
                mime_type: "application/octet-stream".to_string(),
            }),
        );
    }

    fn insert_returned_file_path(&self, requested_path: &str, returned_path: &str) {
        self.results.lock().expect("results").insert(
            requested_path.to_string(),
            Ok(WorkspaceFile {
                path: ScopedPath::new(returned_path).expect("scoped path"),
                filename: returned_path.rsplit('/').next().map(str::to_string),
                mime_type: "application/octet-stream".to_string(),
                bytes: b"data".to_vec(),
            }),
        );
    }
}

#[async_trait]
impl ProjectFilesystemReader for ScriptedProjectFilesystem {
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
        self.results
            .lock()
            .expect("results")
            .get(path)
            .cloned()
            .unwrap_or(Err(ProjectFsError::NotFound))
    }

    async fn stat(
        &self,
        _thread_scope: &ThreadScope,
        path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError> {
        self.stat_calls
            .lock()
            .expect("stat calls")
            .push(path.to_string());
        self.stats
            .lock()
            .expect("stats")
            .get(path)
            .cloned()
            .unwrap_or(Err(ProjectFsError::NotFound))
    }
}

fn sent(reference: &str) -> PartDeliveryOutcome {
    PartDeliveryOutcome::Sent {
        vendor_message_ref: Some(reference.to_string()),
    }
}

fn retryable_part() -> PartDeliveryOutcome {
    PartDeliveryOutcome::Retryable {
        reason: "vendor 429".to_string(),
    }
}

fn coordinator_over(
    store: &Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    adapter: &Arc<ScriptedChannelAdapter>,
) -> DeliveryCoordinator {
    coordinator_over_recording_reply_lookups(store, adapter).0
}

/// Same coordinator, with a handle on the reply-context double so a test can
/// assert *which identity* the anchor was looked up under.
fn coordinator_over_recording_reply_lookups(
    store: &Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    adapter: &Arc<ScriptedChannelAdapter>,
) -> (DeliveryCoordinator, Arc<FixedReplyContext>) {
    let reply_context = Arc::new(FixedReplyContext::new(b"vendor-reply-ctx".to_vec()));
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(store) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(adapter),
            unavailable: false,
        }),
        Arc::clone(&reply_context) as Arc<dyn DeliveryReplyContextSource>,
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    );
    (coordinator, reply_context)
}

/// Resolver that rejects with `OutboundTargetNotDirectMessage` whenever
/// `require_direct_message` is set — the coordinator-path analog of the live
/// `TriggeredReplyTargetAuthority` DM guard (`run_delivery/triggered.rs`),
/// standing in for a non-DM target. Backs
/// `coordinator_require_direct_message_rejects_non_dm_target_without_egress`,
/// which ports the #4953 security pins from the retired
/// `prepare_and_render_product_outbound` DM tests onto the live coordinator.
struct DmRequiringTargetResolver;

#[async_trait]
impl ProductOutboundTargetResolver for DmRequiringTargetResolver {
    async fn resolve_product_outbound_target_metadata(
        &self,
        _target: &ironclaw_outbound::ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductSurfaceFailure> {
        if require_direct_message {
            return Err(ProductSurfaceFailure::OutboundTargetNotDirectMessage);
        }
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: ExternalConversationRef::new(None, "tg-chat-dm", None, None)
                .expect("valid external conversation"),
            external_actor_ref: None,
        })
    }
}

fn coordinated_final_reply<'a>(
    scope: TurnScope,
    extension_id: &'a str,
    thread_scope: &'a ThreadScope,
) -> CoordinatedDeliveryRequest<'a> {
    CoordinatedDeliveryRequest {
        intent: DeliveryIntent::FinalReply,
        delivery: delivery_request(scope),
        parts: vec![
            ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
                "final reply".to_string(),
            ),
        ],
        attachments: Vec::new(),
        thread_anchor: Some("thread-1".to_string()),
        require_direct_message_target: false,
        extension_id,
        thread_scope,
    }
}

async fn coordinate_workspace_reply(
    project_filesystem: &dyn ProjectFilesystemReader,
    text: &str,
    attachments: Vec<AttachmentRef>,
    reports: Vec<Result<DeliveryReport, ChannelError>>,
) -> (
    Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError>,
    Arc<ScriptedChannelAdapter>,
    Arc<OutboundStateStore<InMemoryBackend>>,
    TurnScope,
) {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        reports,
    ));
    let coordinator = coordinator_over(&store, &adapter);
    let thread_scope = project_thread_scope();
    let mut request = coordinated_final_reply(scope.clone(), "vendorx", &thread_scope);
    request.parts =
        vec![ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(text.to_string())];
    request.attachments = attachments;
    let result = coordinator
        .deliver(&policy, &resolver, project_filesystem, request)
        .await;
    (result, adapter, store, scope)
}

fn workspace_attachment_ref(
    id: &str,
    path: &str,
    filename: &str,
    mime_type: &str,
    size_bytes: u64,
) -> AttachmentRef {
    AttachmentRef {
        id: id.to_string(),
        kind: AttachmentKind::from_mime_type(mime_type),
        mime_type: mime_type.to_string(),
        filename: Some(filename.to_string()),
        size_bytes: Some(size_bytes),
        storage_key: Some(path.to_string()),
        extracted_text: None,
    }
}

#[tokio::test]
async fn coordinator_persists_sending_before_the_adapter_delivers() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-100")],
        })],
    ));
    let (coordinator, reply_context) = coordinator_over_recording_reply_lookups(&store, &adapter);

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            &NO_PROJECT_FILESYSTEM,
            coordinated_final_reply(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect("delivery drives");

    let CoordinatedDeliveryOutcome::Delivered {
        attempt,
        conversation,
        vendor_message_refs,
    } = outcome
    else {
        panic!("expected delivered outcome");
    };
    assert_eq!(vendor_message_refs, vec!["ts-100".to_string()]);
    // The resolved target conversation rides the outcome so emitters can
    // record gate routes / cleanup targets without vendor knowledge.
    assert_eq!(conversation.conversation_id(), "tg-chat-123");
    // OUT-3: the adapter observed the attempt already persisted as Sending.
    assert_eq!(
        adapter.observed_statuses(),
        vec![ironclaw_outbound::OutboundDeliveryStatus::Sending]
    );
    // ING-11: the stored reply context rode the envelope back to the adapter.
    let envelopes = adapter.envelopes();
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        envelopes[0].reply_context.as_deref(),
        Some(b"vendor-reply-ctx".as_slice())
    );
    assert_eq!(
        envelopes[0].delivery_attempt_id,
        attempt.delivery_id.to_string()
    );
    assert_eq!(
        envelopes[0].target.thread_anchor.as_deref(),
        Some("thread-1")
    );
    // ING-11 identity: the anchor is looked up under the identity of the
    // channel the resolver returned — extension `vendorx`, installation
    // `inst-1` — not under the requested extension id in both slots. The two
    // are distinct newtypes now, so a transposition is a compile error; this
    // pins that the coordinator reads them off the resolved channel rather
    // than reconstructing either one.
    assert_eq!(
        *reply_context.asked.lock().expect("lock"),
        vec![("vendorx".to_string(), "inst-1".to_string())]
    );
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
}

#[tokio::test]
async fn coordinator_require_direct_message_rejects_non_dm_target_without_egress() {
    // Ported from the retired `prepare_and_render_product_outbound` DM tests
    // (`require_direct_message_true_propagates_to_resolver_and_maps_to_rejected`
    // + its false sibling), born in the #4953 fix "gate triggered Slack OAuth
    // URL on a verified personal DM". The live coordinator must forward
    // `require_direct_message` to the target resolver and, on
    // `OutboundTargetNotDirectMessage`, mark the attempt Failed{Rejected}
    // WITHOUT touching the channel adapter (fail-closed before any vendor
    // egress — OUT-2). The false case (delivers normally) is pinned by
    // `coordinator_persists_sending_before_the_adapter_delivers`, whose request
    // carries `require_direct_message_target: false`.
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = DmRequiringTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-dm")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let thread_scope = project_thread_scope();
    let request = CoordinatedDeliveryRequest {
        intent: DeliveryIntent::FinalReply,
        delivery: delivery_request(scope.clone()),
        parts: vec![
            ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
                "dm only".to_string(),
            ),
        ],
        attachments: Vec::new(),
        thread_anchor: Some("thread-1".to_string()),
        require_direct_message_target: true,
        extension_id: "vendorx",
        thread_scope: &thread_scope,
    };
    let error = coordinator
        .deliver(&policy, &resolver, &NO_PROJECT_FILESYSTEM, request)
        .await
        .expect_err("require_direct_message=true against a non-DM target must reject");
    assert!(
        matches!(
            error,
            CoordinatedDeliveryError::Workflow(
                ProductSurfaceFailure::OutboundTargetNotDirectMessage
            )
        ),
        "unexpected error: {error:?}"
    );
    // Fail-closed BEFORE any vendor egress: the channel adapter never delivered.
    assert_eq!(adapter.deliver_calls(), 0);
    // Audit records Rejected (not Unknown) — the #4953 failure-kind mapping,
    // via `delivery_failure_kind_for_surface_error`.
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed
    );
    assert_eq!(
        attempts[0].failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::Rejected)
    );
}

#[tokio::test]
async fn coordinator_rejected_policy_decision_does_not_reach_the_adapter() {
    // Ported from the retired `revoked_or_rejected_target_does_not_call_render_or_egress`.
    // When the outbound policy rejects the candidate (a revoked/denied reply
    // target — here the validator is deliberately NOT told to `allow` it), the
    // coordinator returns `Rejected` and NEVER reaches the channel adapter
    // (fail-closed before any vendor egress). The failure kind is the policy's
    // AuthorizationRevoked.
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default(); // target not allowed → policy rejects
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-should-not-happen")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            &NO_PROJECT_FILESYSTEM,
            coordinated_final_reply(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect("a policy rejection is a delivery outcome, not a coordinator error");
    let CoordinatedDeliveryOutcome::Rejected { attempt } = outcome else {
        panic!("expected a Rejected outcome, got {outcome:?}");
    };
    assert_eq!(
        attempt.failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::AuthorizationRevoked)
    );
    // Fail-closed: the channel adapter was never reached.
    assert_eq!(adapter.deliver_calls(), 0);
}

#[tokio::test]
async fn coordinator_retries_fully_retryable_reports_then_delivers() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![
            Ok(DeliveryReport {
                parts: vec![retryable_part()],
            }),
            Ok(DeliveryReport {
                parts: vec![sent("ts-200")],
            }),
        ],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            &NO_PROJECT_FILESYSTEM,
            coordinated_final_reply(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect("delivery drives");

    assert!(matches!(
        outcome,
        CoordinatedDeliveryOutcome::Delivered { .. }
    ));
    assert_eq!(adapter.deliver_calls(), 2, "one retry then success");
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
}

#[tokio::test]
async fn coordinator_partial_multipart_failure_is_terminal_without_retry() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-300"), retryable_part()],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            &NO_PROJECT_FILESYSTEM,
            coordinated_final_reply(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect("delivery drives");

    // OUT-7: once any part sent, a later retryable failure is terminal — a
    // whole-envelope retry would duplicate the accepted part.
    assert!(matches!(
        outcome,
        CoordinatedDeliveryOutcome::Failed {
            failure_kind: ironclaw_outbound::DeliveryFailureKind::Rejected,
            ..
        }
    ));
    assert_eq!(adapter.deliver_calls(), 1, "no blind whole-envelope retry");
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed
    );
}

#[tokio::test]
async fn coordinator_workspace_file_partial_send_is_terminal_without_retry() {
    let files = ScriptedProjectFilesystem::default();
    files.insert_file("/workspace/report.pdf", 3);
    let (outcome, adapter, store, scope) = coordinate_workspace_reply(
        &files,
        "report: /workspace/report.pdf",
        vec![workspace_attachment_ref(
            "report",
            "/workspace/report.pdf",
            "final-report.pdf",
            "application/pdf",
            3,
        )],
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-text"), retryable_part()],
        })],
    )
    .await;

    assert!(matches!(
        outcome.expect("delivery outcome"),
        CoordinatedDeliveryOutcome::Failed {
            failure_kind: ironclaw_outbound::DeliveryFailureKind::Rejected,
            ..
        }
    ));
    assert_eq!(
        adapter.deliver_calls(),
        1,
        "file part is never blindly retried"
    );
    assert!(matches!(
        &adapter.envelopes()[0].parts[1],
        ironclaw_extension_contracts::channel_adapter::OutboundPart::File(file)
            if file.path.as_str() == "/workspace/report.pdf"
                && file.filename.as_deref() == Some("final-report.pdf")
                && file.mime_type == "application/pdf"
    ));
    let attempts = store.list_delivery_attempts(scope).await.expect("attempts");
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed
    );
}

#[tokio::test]
async fn coordinator_preserves_text_and_materializes_only_durable_attachment_refs() {
    let files = ScriptedProjectFilesystem::default();
    files.insert_file("/workspace/report.txt", 3);
    files.insert_file("/workspace/data.csv", 4);
    let (outcome, adapter, _, _) = coordinate_workspace_reply(
        &files,
        "Literal [ bracket stays.\nCreated both files:\n\
         1. [Readable report](/workspace/report.txt)\n\
         2. [/workspace/data.csv](/workspace/data.csv)\n\
         3. [Not attached](/workspace/missing.txt)",
        vec![
            workspace_attachment_ref(
                "report",
                "/workspace/report.txt",
                "report.txt",
                "text/plain",
                3,
            ),
            workspace_attachment_ref("data", "/workspace/data.csv", "data.csv", "text/csv", 4),
        ],
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-text"), sent("file-report"), sent("file-data")],
        })],
    )
    .await;

    assert!(matches!(
        outcome,
        Ok(CoordinatedDeliveryOutcome::Delivered { .. })
    ));
    assert!(matches!(
        &adapter.envelopes()[0].parts[0],
        ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(text)
            if text == "Literal [ bracket stays.\nCreated both files:\n\
                1. [Readable report](/workspace/report.txt)\n\
                2. [/workspace/data.csv](/workspace/data.csv)\n\
                3. [Not attached](/workspace/missing.txt)"
    ));
    assert!(matches!(
        &adapter.envelopes()[0].parts[1],
        ironclaw_extension_contracts::channel_adapter::OutboundPart::File(file)
            if file.path.as_str() == "/workspace/report.txt"
                && file.filename.as_deref() == Some("report.txt")
                && file.mime_type == "text/plain"
    ));
    assert!(matches!(
        &adapter.envelopes()[0].parts[2],
        ironclaw_extension_contracts::channel_adapter::OutboundPart::File(file)
            if file.path.as_str() == "/workspace/data.csv"
                && file.filename.as_deref() == Some("data.csv")
                && file.mime_type == "text/csv"
    ));
}

#[tokio::test]
async fn coordinator_does_not_materialize_workspace_path_mentioned_only_in_prose() {
    let files = ScriptedProjectFilesystem::default();
    files.insert_file("/workspace/report.pdf", 3);
    let (outcome, adapter, _, _) = coordinate_workspace_reply(
        &files,
        "The report remains at /workspace/report.pdf.",
        Vec::new(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-text")],
        })],
    )
    .await;

    assert!(matches!(
        outcome,
        Ok(CoordinatedDeliveryOutcome::Delivered { .. })
    ));
    assert_eq!(files.read_count(), 0);
    assert_eq!(adapter.envelopes()[0].parts.len(), 1);
    assert!(matches!(
        &adapter.envelopes()[0].parts[0],
        ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(text)
            if text.contains("/workspace/report.pdf")
    ));
}

#[tokio::test]
async fn coordinator_reads_workspace_only_after_channel_and_reply_context_resolution() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let target_resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-text"), sent("ts-file")],
        })],
    ));
    let phase = Arc::new(AtomicU8::new(0));
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn OutboundStateStorePort>,
        Arc::new(OrderedChannelResolver {
            adapter: Arc::clone(&adapter),
            phase: Arc::clone(&phase),
        }),
        Arc::new(OrderedReplyContext {
            phase: Arc::clone(&phase),
        }),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: std::time::Duration::ZERO,
        },
    );
    let files = OrderedProjectFilesystem {
        phase: Arc::clone(&phase),
    };
    let thread_scope = project_thread_scope();
    let mut request = coordinated_final_reply(scope, "vendorx", &thread_scope);
    request.parts = vec![
        ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
            "report: /workspace/ordered.pdf".to_string(),
        ),
    ];
    request.attachments = vec![workspace_attachment_ref(
        "ordered",
        "/workspace/ordered.pdf",
        "ordered.pdf",
        "application/pdf",
        7,
    )];

    let outcome = coordinator
        .deliver(&policy, &target_resolver, &files, request)
        .await
        .expect("delivery succeeds in the required order");

    assert!(matches!(
        outcome,
        CoordinatedDeliveryOutcome::Delivered { .. }
    ));
    assert_eq!(phase.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn coordinator_rejects_caller_supplied_file_parts_before_policy_or_egress() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = coordinator_over(&store, &adapter);
    let thread_scope = project_thread_scope();
    let mut request = coordinated_final_reply(scope.clone(), "vendorx", &thread_scope);
    request.parts.push(
        ironclaw_extension_contracts::channel_adapter::OutboundPart::File(WorkspaceFile {
            path: ScopedPath::new("/workspace/untrusted.bin").expect("scoped path"),
            filename: Some("untrusted.bin".to_string()),
            mime_type: "application/octet-stream".to_string(),
            bytes: vec![0; 1],
        }),
    );

    let error = coordinator
        .deliver(&policy, &resolver, &NO_PROJECT_FILESYSTEM, request)
        .await
        .expect_err("caller-supplied file values must fail closed");

    assert!(matches!(
        error,
        CoordinatedDeliveryError::PreMaterializedWorkspaceAttachment
    ));
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("attempts")
            .is_empty(),
        "rejection happens before policy persists an attempt"
    );
    assert_eq!(adapter.deliver_calls(), 0);
}

#[tokio::test]
async fn coordinator_rejects_pre_materialized_files_on_notice_path() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = coordinator_over(&store, &adapter);
    let mut request = working_notice(scope.clone(), "vendorx");
    request.parts.push(
        ironclaw_extension_contracts::channel_adapter::OutboundPart::File(WorkspaceFile {
            path: ScopedPath::new("/workspace/untrusted.bin").expect("scoped path"),
            filename: Some("untrusted.bin".to_string()),
            mime_type: "application/octet-stream".to_string(),
            bytes: vec![0; 1],
        }),
    );

    let error = coordinator
        .deliver_notice(request)
        .await
        .expect_err("notice callers cannot inject file bytes");

    assert!(matches!(
        error,
        CoordinatedDeliveryError::PreMaterializedWorkspaceAttachment
    ));
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("attempts")
            .is_empty()
    );
    assert_eq!(adapter.deliver_calls(), 0);
}

#[tokio::test]
async fn coordinator_fails_closed_when_workspace_file_is_missing_or_denied() {
    for (path, expected) in [
        ("/workspace/missing.pdf", ProjectFsError::NotFound),
        ("/workspace/secret.pdf", ProjectFsError::Denied),
    ] {
        let files = ScriptedProjectFilesystem::default();
        files.insert_error(path, expected.clone());
        let (outcome, adapter, store, scope) = coordinate_workspace_reply(
            &files,
            &format!("attachment: {path}"),
            vec![workspace_attachment_ref(
                "unavailable",
                path,
                "document.pdf",
                "application/pdf",
                4,
            )],
            Vec::new(),
        )
        .await;

        assert!(matches!(
            outcome,
            Err(CoordinatedDeliveryError::WorkspaceAttachmentRead(error)) if error == expected
        ));
        assert_eq!(
            adapter.deliver_calls(),
            0,
            "adapter must not see partial files"
        );
        let attempts = store.list_delivery_attempts(scope).await.expect("attempts");
        assert_eq!(
            attempts[0].status,
            ironclaw_outbound::OutboundDeliveryStatus::Failed
        );
    }
}

#[tokio::test]
async fn coordinator_classifies_unavailable_workspace_reader_as_transport_unavailable() {
    let files = ScriptedProjectFilesystem::default();
    files.insert_error("/workspace/report.pdf", ProjectFsError::Unavailable);
    let (outcome, adapter, store, scope) = coordinate_workspace_reply(
        &files,
        "attachment: /workspace/report.pdf",
        vec![workspace_attachment_ref(
            "report",
            "/workspace/report.pdf",
            "report.pdf",
            "application/pdf",
            4,
        )],
        Vec::new(),
    )
    .await;

    assert!(matches!(
        outcome,
        Err(CoordinatedDeliveryError::WorkspaceAttachmentRead(
            ProjectFsError::Unavailable
        ))
    ));
    assert_eq!(adapter.deliver_calls(), 0);
    let attempts = store.list_delivery_attempts(scope).await.expect("attempts");
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed
    );
    assert_eq!(
        attempts[0].failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::TransportUnavailable)
    );
}

#[tokio::test]
async fn coordinator_enforces_workspace_file_count_per_file_and_total_budgets() {
    let too_many_refs = (0..=DEFAULT_ATTACHMENT_BUDGETS.max_count)
        .map(|index| {
            workspace_attachment_ref(
                &format!("file-{index}"),
                &format!("/workspace/file-{index}.txt"),
                &format!("file-{index}.txt"),
                "text/plain",
                1,
            )
        })
        .collect::<Vec<_>>();
    let files = ScriptedProjectFilesystem::default();
    let (outcome, adapter, _, _) =
        coordinate_workspace_reply(&files, "too many files", too_many_refs, Vec::new()).await;
    assert!(matches!(
        outcome,
        Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded)
    ));
    assert_eq!(files.read_count(), 0, "count is rejected before any read");
    assert_eq!(adapter.deliver_calls(), 0);

    let files = ScriptedProjectFilesystem::default();
    files.insert_file(
        "/workspace/oversize.bin",
        DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes + 1,
    );
    let (outcome, adapter, _, _) = coordinate_workspace_reply(
        &files,
        "oversized attachment",
        vec![workspace_attachment_ref(
            "oversize",
            "/workspace/oversize.bin",
            "oversize.bin",
            "application/octet-stream",
            (DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes + 1) as u64,
        )],
        Vec::new(),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded)
    ));
    assert_eq!(
        files.read_count(),
        0,
        "oversized metadata must reject before allocating file bytes"
    );
    assert_eq!(adapter.deliver_calls(), 0);

    let files = ScriptedProjectFilesystem::default();
    let each = 4 * 1024 * 1024;
    for path in [
        "/workspace/one.bin",
        "/workspace/two.bin",
        "/workspace/three.bin",
    ] {
        files.insert_file(path, each);
    }
    let (outcome, adapter, _, _) = coordinate_workspace_reply(
        &files,
        "three files",
        vec![
            workspace_attachment_ref(
                "one",
                "/workspace/one.bin",
                "one.bin",
                "application/octet-stream",
                each as u64,
            ),
            workspace_attachment_ref(
                "two",
                "/workspace/two.bin",
                "two.bin",
                "application/octet-stream",
                each as u64,
            ),
            workspace_attachment_ref(
                "three",
                "/workspace/three.bin",
                "three.bin",
                "application/octet-stream",
                each as u64,
            ),
        ],
        Vec::new(),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded)
    ));
    assert_eq!(adapter.deliver_calls(), 0);
}

#[tokio::test]
async fn coordinator_rejects_stat_and_read_path_mismatches() {
    let files = ScriptedProjectFilesystem::default();
    files.insert_file("/workspace/report.pdf", 4);
    files.insert_stat_path("/workspace/report.pdf", "/workspace/other.pdf", 4);
    let (outcome, adapter, _, _) = coordinate_workspace_reply(
        &files,
        "report",
        vec![workspace_attachment_ref(
            "report",
            "/workspace/report.pdf",
            "report.pdf",
            "application/pdf",
            4,
        )],
        Vec::new(),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(CoordinatedDeliveryError::WorkspaceAttachmentRead(
            ProjectFsError::Internal
        ))
    ));
    assert_eq!(files.read_count(), 0, "stat mismatch rejects before read");
    assert_eq!(adapter.deliver_calls(), 0);

    let files = ScriptedProjectFilesystem::default();
    files.insert_file("/workspace/report.pdf", 4);
    files.insert_returned_file_path("/workspace/report.pdf", "/workspace/other.pdf");
    let (outcome, adapter, _, _) = coordinate_workspace_reply(
        &files,
        "report",
        vec![workspace_attachment_ref(
            "report",
            "/workspace/report.pdf",
            "report.pdf",
            "application/pdf",
            4,
        )],
        Vec::new(),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(CoordinatedDeliveryError::WorkspaceAttachmentRead(
            ProjectFsError::Internal
        ))
    ));
    assert_eq!(files.read_count(), 1);
    assert_eq!(adapter.deliver_calls(), 0);
}

/// Delegating store whose terminal `Delivered` status write fails — the
/// durable shape behind theredspoon's #7157 flag (and #7029's fix on main):
/// vendor egress succeeded, but the confirmation row never committed.
struct TerminalDeliveredWriteFailingStore {
    inner: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
}

#[async_trait]
impl OutboundStateStorePort for TerminalDeliveredWriteFailingStore {
    async fn put_run_delivery_cleanup(
        &self,
        record: ironclaw_outbound::RunDeliveryCleanupRecord,
    ) -> Result<(), OutboundError> {
        self.inner.put_run_delivery_cleanup(record).await
    }
    async fn load_run_delivery_cleanup(
        &self,
        request: ironclaw_outbound::RunDeliveryCleanupRequest,
    ) -> Result<Vec<ironclaw_outbound::RunDeliveryCleanupRecord>, OutboundError> {
        self.inner.load_run_delivery_cleanup(request).await
    }
    async fn complete_run_delivery_cleanup(
        &self,
        record: &ironclaw_outbound::RunDeliveryCleanupRecord,
    ) -> Result<(), OutboundError> {
        self.inner.complete_run_delivery_cleanup(record).await
    }
    async fn put_thread_notification_policy(
        &self,
        policy: ironclaw_outbound::ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        self.inner.put_thread_notification_policy(policy).await
    }
    async fn load_thread_notification_policy(
        &self,
        scope: ironclaw_turns::TurnScope,
    ) -> Result<ironclaw_outbound::ThreadNotificationPolicy, OutboundError> {
        self.inner.load_thread_notification_policy(scope).await
    }
    async fn upsert_subscription(
        &self,
        record: ironclaw_outbound::ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        self.inner.upsert_subscription(record).await
    }
    async fn load_subscription_cursor(
        &self,
        request: ironclaw_outbound::LoadSubscriptionCursorRequest,
    ) -> Result<Option<ironclaw_event_projections::ProjectionCursor>, OutboundError> {
        self.inner.load_subscription_cursor(request).await
    }
    async fn advance_subscription_cursor(
        &self,
        request: ironclaw_outbound::AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        self.inner.advance_subscription_cursor(request).await
    }
    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        self.inner.record_delivery_attempt(attempt).await
    }
    async fn claim_delivery_attempt_for_send(
        &self,
        request: ironclaw_outbound::ClaimDeliveryAttemptForSendRequest,
    ) -> Result<bool, OutboundError> {
        self.inner.claim_delivery_attempt_for_send(request).await
    }
    async fn recover_interrupted_delivery_attempt(
        &self,
        request: ironclaw_outbound::RecoverInterruptedDeliveryRequest,
    ) -> Result<bool, OutboundError> {
        self.inner
            .recover_interrupted_delivery_attempt(request)
            .await
    }
    async fn update_delivery_status(
        &self,
        request: ironclaw_outbound::UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        if matches!(
            request.status,
            ironclaw_outbound::OutboundDeliveryStatus::Delivered
        ) {
            return Err(OutboundError::Backend);
        }
        self.inner.update_delivery_status(request).await
    }
    async fn list_delivery_attempts(
        &self,
        scope: ironclaw_turns::TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        self.inner.list_delivery_attempts(scope).await
    }
}

/// theredspoon's #7157 flag: `mark_terminal` used to swallow a failed durable
/// `Delivered` write and the coordinator still reported `Delivered` — a
/// fabricated success contradicting the durability guarantee. The vendor send
/// DID happen, so the honest outcome keeps the provider refs but reports the
/// confirmation as unconfirmed (never an error: an error would invite a
/// duplicate resend).
#[tokio::test]
async fn coordinator_does_not_report_delivered_when_the_terminal_write_fails() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-777")],
        })],
    ));
    let reply_context = Arc::new(FixedReplyContext::new(b"vendor-reply-ctx".to_vec()));
    let coordinator = DeliveryCoordinator::new(
        Arc::new(TerminalDeliveredWriteFailingStore {
            inner: Arc::clone(&store),
        }) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: false,
        }),
        reply_context as Arc<dyn DeliveryReplyContextSource>,
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    );

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            &NO_PROJECT_FILESYSTEM,
            coordinated_final_reply(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect("delivery drives");

    match outcome {
        CoordinatedDeliveryOutcome::DeliveredUnconfirmed {
            vendor_message_refs,
            ..
        } => {
            assert_eq!(vendor_message_refs, vec!["ts-777".to_string()]);
        }
        other => panic!(
            "a failed terminal Delivered write must yield DeliveredUnconfirmed, got {other:?}"
        ),
    }
    // The durable row never reached Delivered — it must not read as confirmed.
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(
        !matches!(
            attempts[0].status,
            ironclaw_outbound::OutboundDeliveryStatus::Delivered
        ),
        "durable status must not be Delivered when the confirmation write failed: {:?}",
        attempts[0].status
    );
}

#[tokio::test]
async fn coordinator_recovery_marks_interrupted_sending_attempts_unknown() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-400")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);
    coordinator
        .deliver(
            &policy,
            &resolver,
            &NO_PROJECT_FILESYSTEM,
            coordinated_final_reply(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect("delivery drives");
    // Rewind the delivered attempt to Sending — the durable shape a crash
    // between vendor egress and the result write leaves behind.
    let attempts = store.list_delivery_attempts(scope.clone()).await.unwrap();
    store
        .update_delivery_status(ironclaw_outbound::UpdateDeliveryStatusRequest {
            delivery_id: attempts[0].delivery_id,
            scope: scope.clone(),
            status: ironclaw_outbound::OutboundDeliveryStatus::Sending,
            updated_at: Utc::now(),
            failure_kind: None,
        })
        .await
        .unwrap();

    let recovered = coordinator
        .recover_interrupted_deliveries(scope.clone())
        .await
        .expect("recovery scans");
    assert_eq!(recovered, 1);
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    // OUT-6: terminal-ambiguous, never blindly resent.
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Unknown
    );
    assert_eq!(adapter.deliver_calls(), 1, "adapter never called again");
}

#[tokio::test]
async fn coordinator_fails_closed_when_the_channel_is_unavailable() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: true,
        }),
        Arc::new(FixedReplyContext::new(Vec::new())),
        DeliveryRetryPolicy::default(),
    );

    let error = coordinator
        .deliver(
            &policy,
            &resolver,
            &NO_PROJECT_FILESYSTEM,
            coordinated_final_reply(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect_err("unavailable channel fails closed");
    assert!(matches!(
        error,
        CoordinatedDeliveryError::ChannelUnavailable { .. }
    ));
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed
    );
    assert_eq!(
        attempts[0].failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::TransportUnavailable)
    );
    assert_eq!(adapter.deliver_calls(), 0);
}

// ── Notice-class deliveries (§5.4: Working / Cleanup / FailureNotice /
// ConnectRequired) — source-routed system notices on the originating
// conversation; no outbound-policy resolution, but the same persistence,
// retry, and sole-writer rules apply. ──────────────────────────────────────

fn notice_source_conversation() -> ExternalConversationRef {
    ExternalConversationRef::new(Some("team-9"), "conv-notice", Some("1719.100"), None)
        .expect("valid notice conversation")
}

fn working_notice(scope: TurnScope, extension_id: &str) -> NoticeDeliveryRequest<'_> {
    NoticeDeliveryRequest {
        intent: DeliveryIntent::Working,
        scope,
        turn_run_id: None,
        conversation: notice_source_conversation(),
        thread_anchor: Some("1719.100".to_string()),
        parts: vec![
            ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
                "Working on it...".to_string(),
            ),
        ],
        extension_id,
        notice_ref: "run-42".to_string(),
    }
}

#[tokio::test]
async fn coordinator_notice_is_source_routed_and_persists_before_egress() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-900")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let outcome = coordinator
        .deliver_notice(working_notice(scope.clone(), "vendorx"))
        .await
        .expect("notice delivers");

    let CoordinatedDeliveryOutcome::Delivered {
        attempt,
        conversation,
        vendor_message_refs,
    } = outcome
    else {
        panic!("expected delivered outcome");
    };
    assert_eq!(vendor_message_refs, vec!["ts-900".to_string()]);
    assert_eq!(
        conversation.conversation_fingerprint(),
        notice_source_conversation().conversation_fingerprint()
    );
    // OUT-3 applies to notices too: `Sending` durable before the adapter ran.
    assert_eq!(
        adapter.observed_statuses(),
        vec![ironclaw_outbound::OutboundDeliveryStatus::Sending]
    );
    let envelopes = adapter.envelopes();
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        envelopes[0].target.conversation.conversation_fingerprint(),
        notice_source_conversation().conversation_fingerprint()
    );
    assert_eq!(
        envelopes[0].target.thread_anchor.as_deref(),
        Some("1719.100")
    );
    // The stored source reply context rides back on notice envelopes too
    // (ING-11 covers system notices).
    assert_eq!(
        envelopes[0].reply_context.as_deref(),
        Some(b"vendor-reply-ctx".as_slice())
    );
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].delivery_id, attempt.delivery_id);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
    assert_eq!(
        attempts[0].candidate.kind,
        ironclaw_outbound::OutboundPushKind::DeliveryStatus
    );
    assert!(!attempts[0].candidate.requires_reply_target_revalidation);
}

#[tokio::test]
async fn coordinator_notice_rejects_policy_class_intents() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = coordinator_over(&store, &adapter);

    for intent in [DeliveryIntent::FinalReply, DeliveryIntent::ModelDelivery] {
        let mut request = working_notice(scope.clone(), "vendorx");
        request.intent = intent;
        let error = coordinator
            .deliver_notice(request)
            .await
            .expect_err("policy-class intents must use the policy path");
        assert!(matches!(
            error,
            CoordinatedDeliveryError::IntentClassMismatch { .. }
        ));
    }
    assert_eq!(adapter.deliver_calls(), 0);
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn coordinator_deliver_rejects_notice_class_intents() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let thread_scope = project_thread_scope();
    let mut request = coordinated_final_reply(scope.clone(), "vendorx", &thread_scope);
    request.intent = DeliveryIntent::Working;
    let error = coordinator
        .deliver(&policy, &resolver, &NO_PROJECT_FILESYSTEM, request)
        .await
        .expect_err("notice-class intents must use the notice path");
    assert!(matches!(
        error,
        CoordinatedDeliveryError::IntentClassMismatch { .. }
    ));
    assert_eq!(adapter.deliver_calls(), 0);
}

#[test]
fn model_delivery_is_policy_class() {
    use ironclaw_assistant::DeliveryIntent;
    assert!(DeliveryIntent::ModelDelivery.runs_outbound_policy());
    assert!(!DeliveryIntent::ModelDelivery.is_notice_class());
}

#[tokio::test]
async fn coordinator_cleanup_retract_parts_reach_the_adapter() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![PartDeliveryOutcome::Sent {
                vendor_message_ref: None,
            }],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let mut request = working_notice(scope.clone(), "vendorx");
    request.intent = DeliveryIntent::Cleanup;
    request.parts = vec![
        ironclaw_extension_contracts::channel_adapter::OutboundPart::Retract {
            vendor_message_ref: "ts-900".to_string(),
        },
    ];
    let outcome = coordinator
        .deliver_notice(request)
        .await
        .expect("cleanup delivers");
    assert!(matches!(
        outcome,
        CoordinatedDeliveryOutcome::Delivered { .. }
    ));
    let envelopes = adapter.envelopes();
    assert_eq!(envelopes.len(), 1);
    assert!(matches!(
        &envelopes[0].parts[..],
        [ironclaw_extension_contracts::channel_adapter::OutboundPart::Retract { vendor_message_ref }]
            if vendor_message_ref == "ts-900"
    ));
}

#[tokio::test]
async fn coordinator_lazily_recovers_interrupted_attempts_before_a_scopes_first_delivery() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    // Durable shape a crash leaves behind: an attempt stuck in `Sending`
    // from a PREVIOUS process lifetime.
    let stray = OutboundDeliveryAttempt {
        delivery_id: ironclaw_outbound::OutboundDeliveryId::new(),
        scope: scope.clone(),
        candidate: ironclaw_outbound::OutboundPushCandidate {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
            turn_run_id: None,
            target: validated_reply_target(),
            kind: ironclaw_outbound::OutboundPushKind::DeliveryStatus,
            projection_ref: ironclaw_outbound::ProjectionUpdateRef::new("projection:stray")
                .expect("projection ref"),
            requires_reply_target_revalidation: false,
        },
        status: ironclaw_outbound::OutboundDeliveryStatus::Sending,
        attempted_at: Utc::now(),
        failure_kind: None,
    };
    store
        .record_delivery_attempt(stray.clone())
        .await
        .expect("seed stray attempt");

    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-950")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);
    coordinator
        .deliver_notice(working_notice(scope.clone(), "vendorx"))
        .await
        .expect("notice delivers");

    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    let recovered = attempts
        .iter()
        .find(|attempt| attempt.delivery_id == stray.delivery_id)
        .expect("stray attempt still present");
    // OUT-6: found in Sending from a prior lifetime → Unknown, never resent.
    assert_eq!(
        recovered.status,
        ironclaw_outbound::OutboundDeliveryStatus::Unknown
    );
    assert_eq!(adapter.deliver_calls(), 1, "only the new notice was sent");
}

#[tokio::test]
async fn coordinator_notice_fails_closed_when_the_channel_is_unavailable() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: true,
        }),
        Arc::new(FixedReplyContext::new(Vec::new())),
        DeliveryRetryPolicy::default(),
    );

    let error = coordinator
        .deliver_notice(working_notice(scope.clone(), "vendorx"))
        .await
        .expect_err("unavailable channel fails closed");
    assert!(matches!(
        error,
        CoordinatedDeliveryError::ChannelUnavailable { .. }
    ));
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed
    );
    assert_eq!(adapter.deliver_calls(), 0);
}

/// A codec whose DM verdict is configurable, so the DM rule can be driven
/// through the REAL resolver rather than a double that pre-decides it.
struct ConfigurableDmCodec {
    direct_message: bool,
}

impl ironclaw_extension_contracts::preference_target::PreferenceTargetCodec
    for ConfigurableDmCodec
{
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef> {
        let conversation = target.as_str().strip_prefix("reply:dm-codec:")?;
        ExternalConversationRef::new(None::<&str>, conversation, None, None).ok()
    }

    fn is_personal_direct_message(&self, _target: &ReplyTargetBindingRef) -> bool {
        self.direct_message
    }

    fn direct_message_actor_for_target(&self, _target: &ReplyTargetBindingRef) -> Option<String> {
        None
    }

    fn encode_shared_conversation_target(
        &self,
        _request: ironclaw_extension_contracts::preference_target::PreferenceTargetEncodeRequest<
            '_,
        >,
    ) -> Option<ReplyTargetBindingRef> {
        None
    }

    fn encode_personal_direct_message_target(
        &self,
        _request: ironclaw_extension_contracts::preference_target::PreferenceTargetEncodeRequest<
            '_,
        >,
        _external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef> {
        None
    }
}

/// The OAuth DM rule, driven through the production resolver.
///
/// `coordinator_require_direct_message_rejects_non_dm_target_without_egress`
/// above pins the coordinator's half with a double that decides the verdict
/// itself, so it cannot catch a resolver that stops consulting
/// `is_personal_direct_message`. The vendor codecs pin the predicate in
/// isolation. Nothing joined the two — the wiring that actually enforces
/// "an OAuth authorization URL only ever lands in a personal DM" was
/// unguarded, and it is now the single enforcement point for both the
/// notifier and `builtin.outbound_deliver`.
#[tokio::test]
async fn codec_resolver_enforces_the_dm_rule_from_the_codec_verdict() {
    for (direct_message, expect_delivery) in [(false, false), (true, true)] {
        let scope = scope();
        let store =
            Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
        let validator = FakeReplyTargetBindingValidator::default();
        validator.allow(
            ReplyTargetBindingRef::new("reply:dm-codec:conv-dm").expect("valid binding ref"),
        );
        let preferences = FakePreferenceRepository::default();
        seed_preference(&preferences, &scope);
        let policy = configured_policy(&store, &validator);
        let adapter = Arc::new(ScriptedChannelAdapter::new(
            Arc::clone(&store),
            scope.clone(),
            vec![Ok(DeliveryReport {
                parts: vec![sent("ts-dm")],
            })],
        ));
        let coordinator = coordinator_over(&store, &adapter);
        let resolver = ironclaw_assistant::CodecChannelTargetResolver::new(vec![Arc::new(
            ConfigurableDmCodec { direct_message },
        )]);

        let thread_scope = project_thread_scope();
        let mut delivery = delivery_request(scope.clone());
        delivery.resolution_request.intent =
            CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: RunNotificationEventKind::AuthRequired,
                origin: RunNotificationOrigin::RunScopedTarget {
                    target: ReplyTargetBindingRef::new("reply:dm-codec:conv-dm")
                        .expect("valid binding ref"),
                },
            });
        let request = CoordinatedDeliveryRequest {
            intent: DeliveryIntent::AuthPrompt,
            delivery,
            parts: vec![
                ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
                    "https://example.test/oauth?code=secret".to_string(),
                ),
            ],
            attachments: Vec::new(),
            thread_anchor: None,
            require_direct_message_target: true,
            extension_id: "vendorx",
            thread_scope: &thread_scope,
        };

        let outcome = coordinator
            .deliver(&policy, &resolver, &NO_PROJECT_FILESYSTEM, request)
            .await;

        if expect_delivery {
            outcome.expect("a personal DM target must accept the authorization URL");
            assert_eq!(
                adapter.deliver_calls(),
                1,
                "a DM target must reach the vendor adapter"
            );
        } else {
            let error = outcome.expect_err("a non-DM target must reject the authorization URL");
            assert!(
                matches!(
                    error,
                    CoordinatedDeliveryError::Workflow(
                        ProductSurfaceFailure::OutboundTargetNotDirectMessage
                    )
                ),
                "unexpected error: {error:?}"
            );
            assert_eq!(
                adapter.deliver_calls(),
                0,
                "an OAuth URL must never reach a vendor adapter for a non-DM target"
            );
        }
    }
}
