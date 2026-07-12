use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    CommunicationPreferenceKey, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    CommunicationPreferenceVersion, DeliveryDefaultScope, InMemoryOutboundStateStore,
    LoadSubscriptionCursorRequest, OutboundDeliveryAttempt, OutboundError, OutboundPolicyService,
    OutboundStateStore, ProjectionSubscriptionRecord, ReplyTargetBindingClaim,
    ReplyTargetBindingValidator, RequestedOutboundContext, RequestedOutboundKind,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin, SystemEventReasonCode,
    ThreadNotificationPolicy, ThreadProjectionAccessClaim, ThreadProjectionAccessPolicy,
    ThreadProjectionAccessRequest, TriggerFireSlot, TriggerOriginRef, TriggerSourceKind,
    UpdateDeliveryStatusRequest, VersionedCommunicationPreferenceRecord,
    WriteCommunicationPreferenceRequest,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeliveryStatus, EgressCredentialHandle, EgressResponse,
    ExternalActorRef, ExternalConversationRef, FakeOutboundDeliverySink, FakeProtocolHttpEgress,
    FinalReplyView, OutboundDeliverySink, ProductAdapter, ProductAdapterCapabilities,
    ProductAdapterError, ProductAdapterId, ProductOutboundEnvelope, ProductOutboundPayload,
    ProductRenderOutcome, ProductSurfaceKind, ProductSynchronousResponse,
    ProductWorkflowRejectionKind, ProgressKind, ProgressUpdateView, ProjectionCursor,
    ProtocolHttpEgress, RedactedString,
};
use ironclaw_product_workflow::{
    ProductOutboundDeliveryOutcome, ProductOutboundDeliveryRequest,
    ProductOutboundStatusUpdateFailure, ProductOutboundTargetResolver, ProductWorkflowError,
    VerifiedProductOutboundTargetMetadata, prepare_and_render_product_outbound,
};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TelegramV2Adapter, TelegramV2AdapterConfig,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};

static SYNC_ADAPTER_CAPABILITIES: LazyLock<ProductAdapterCapabilities> =
    LazyLock::new(ProductAdapterCapabilities::empty);

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
    required_actor: Mutex<Option<TurnActor>>,
    required_modality: Mutex<Option<CommunicationModality>>,
    calls: Mutex<Vec<ReplyTargetBindingRef>>,
}

impl FakeReplyTargetBindingValidator {
    fn allow(&self, target: ReplyTargetBindingRef) {
        self.allowed_targets
            .lock()
            .expect("validator lock")
            .insert(target);
    }

    fn calls(&self) -> usize {
        self.calls.lock().expect("validator lock").len()
    }

    fn require_actor(&self, actor: TurnActor) {
        *self.required_actor.lock().expect("validator lock") = Some(actor);
    }

    fn require_modality(&self, modality: CommunicationModality) {
        *self.required_modality.lock().expect("validator lock") = Some(modality);
    }
}

#[async_trait]
impl ReplyTargetBindingValidator for FakeReplyTargetBindingValidator {
    async fn validate_reply_target(
        &self,
        request: ironclaw_outbound::ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        self.calls
            .lock()
            .expect("validator lock")
            .push(request.candidate.target.clone());
        if self
            .required_actor
            .lock()
            .expect("validator lock")
            .as_ref()
            .is_some_and(|actor| actor != &request.actor)
        {
            return Err(OutboundError::AccessDenied);
        }
        if self
            .required_modality
            .lock()
            .expect("validator lock")
            .is_some_and(|modality| modality != request.modality)
        {
            return Err(OutboundError::AccessDenied);
        }
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
    load_calls: Mutex<usize>,
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

    fn load_calls(&self) -> usize {
        *self.load_calls.lock().expect("preference lock")
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
        *self.load_calls.lock().expect("preference lock") += 1;
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

#[derive(Default)]
struct FakeProductOutboundTargetResolver {
    calls: Mutex<Vec<ReplyTargetBindingRef>>,
    error: Mutex<Option<ProductWorkflowError>>,
}

impl FakeProductOutboundTargetResolver {
    fn calls(&self) -> usize {
        self.calls.lock().expect("target resolver lock").len()
    }

    fn called_targets(&self) -> Vec<ReplyTargetBindingRef> {
        self.calls.lock().expect("target resolver lock").clone()
    }

    fn fail(&self) {
        self.fail_with(ProductWorkflowError::Transient {
            reason: "target metadata unavailable".into(),
        });
    }

    fn fail_with(&self, error: ProductWorkflowError) {
        *self.error.lock().expect("target resolver lock") = Some(error);
    }
}

#[async_trait]
impl ProductOutboundTargetResolver for FakeProductOutboundTargetResolver {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ironclaw_outbound::ValidatedReplyTargetBinding,
        _require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        self.calls
            .lock()
            .expect("target resolver lock")
            .push(target.target().clone());
        if let Some(error) = self.error.lock().expect("target resolver lock").clone() {
            return Err(error);
        }
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

/// A resolver that returns `OutboundTargetNotDirectMessage` when
/// `require_direct_message == true`, and succeeds otherwise.  Used to verify
/// that the `require_direct_message_target` flag threads from the delivery
/// request all the way through to the resolver and produces the right
/// `DeliveryFailureKind::Rejected` audit classification.
#[derive(Default)]
struct DmRequiringFakeProductOutboundTargetResolver {
    calls: Mutex<Vec<(ReplyTargetBindingRef, bool)>>,
}

impl DmRequiringFakeProductOutboundTargetResolver {
    fn calls(&self) -> Vec<(ReplyTargetBindingRef, bool)> {
        self.calls.lock().expect("dm resolver lock").clone()
    }
}

#[async_trait]
impl ProductOutboundTargetResolver for DmRequiringFakeProductOutboundTargetResolver {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ironclaw_outbound::ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        self.calls
            .lock()
            .expect("dm resolver lock")
            .push((target.target().clone(), require_direct_message));
        if require_direct_message {
            return Err(ProductWorkflowError::OutboundTargetNotDirectMessage);
        }
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: ExternalConversationRef::new(None, "tg-chat-dm", None, None)
                .expect("valid external conversation"),
            external_actor_ref: None,
        })
    }
}

#[derive(Default)]
struct StatusFailingOutboundStore {
    inner: InMemoryOutboundStateStore,
    status_update_requests: Mutex<Vec<UpdateDeliveryStatusRequest>>,
}

impl StatusFailingOutboundStore {
    fn status_update_requests(&self) -> Vec<UpdateDeliveryStatusRequest> {
        self.status_update_requests
            .lock()
            .expect("status update lock")
            .clone()
    }
}

#[async_trait]
impl CommunicationPreferenceRepository for StatusFailingOutboundStore {
    async fn put_communication_preference(
        &self,
        record: CommunicationPreferenceRecord,
    ) -> Result<(), OutboundError> {
        self.inner.put_communication_preference(record).await
    }

    async fn load_communication_preference(
        &self,
        key: CommunicationPreferenceKey,
    ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError> {
        self.inner.load_communication_preference(key).await
    }

    async fn write_communication_preference(
        &self,
        request: WriteCommunicationPreferenceRequest,
    ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
        self.inner.write_communication_preference(request).await
    }
}

#[async_trait]
impl OutboundStateStore for StatusFailingOutboundStore {
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

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        self.inner.upsert_subscription(record).await
    }

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
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

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        self.status_update_requests
            .lock()
            .expect("status update lock")
            .push(request);
        Err(OutboundError::Backend)
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        self.inner.list_delivery_attempts(scope).await
    }
}

struct SynchronousResponseAdapter {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
}

impl SynchronousResponseAdapter {
    fn new() -> Self {
        Self {
            adapter_id: ProductAdapterId::new("sync_test").expect("valid adapter id"),
            installation_id: AdapterInstallationId::new("sync_install").expect("valid install"),
        }
    }
}

#[async_trait]
impl ProductAdapter for SynchronousResponseAdapter {
    fn adapter_id(&self) -> &ProductAdapterId {
        &self.adapter_id
    }

    fn installation_id(&self) -> &AdapterInstallationId {
        &self.installation_id
    }

    fn surface_kind(&self) -> ProductSurfaceKind {
        ProductSurfaceKind::SynchronousApi
    }

    fn capabilities(&self) -> &ProductAdapterCapabilities {
        &SYNC_ADAPTER_CAPABILITIES
    }

    fn auth_requirement(&self) -> &AuthRequirement {
        static AUTH_REQUIREMENT: AuthRequirement = AuthRequirement::BearerToken;
        &AUTH_REQUIREMENT
    }

    fn parse_inbound(
        &self,
        _raw_payload: &[u8],
        _auth_evidence: &ironclaw_product_adapters::ProtocolAuthEvidence,
    ) -> Result<ironclaw_product_adapters::ParsedProductInbound, ProductAdapterError> {
        Err(ProductAdapterError::Internal {
            detail: ironclaw_product_adapters::RedactedString::new("not used"),
        })
    }

    async fn render_outbound(
        &self,
        _envelope: ProductOutboundEnvelope,
        _egress: &dyn ProtocolHttpEgress,
        _delivery_sink: &dyn OutboundDeliverySink,
    ) -> Result<ProductRenderOutcome, ProductAdapterError> {
        Ok(ProductRenderOutcome::SynchronousResponse(
            ProductSynchronousResponse {
                content_type: "application/json".into(),
                body: br#"{"ok":true}"#.to_vec(),
            },
        ))
    }
}

struct DeferredAdapter {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
}

impl DeferredAdapter {
    fn new() -> Self {
        Self {
            adapter_id: ProductAdapterId::new("deferred_test").expect("valid adapter id"),
            installation_id: AdapterInstallationId::new("deferred_install").expect("valid install"),
        }
    }
}

#[async_trait]
impl ProductAdapter for DeferredAdapter {
    fn adapter_id(&self) -> &ProductAdapterId {
        &self.adapter_id
    }

    fn installation_id(&self) -> &AdapterInstallationId {
        &self.installation_id
    }

    fn surface_kind(&self) -> ProductSurfaceKind {
        ProductSurfaceKind::SynchronousApi
    }

    fn capabilities(&self) -> &ProductAdapterCapabilities {
        &SYNC_ADAPTER_CAPABILITIES
    }

    fn auth_requirement(&self) -> &AuthRequirement {
        static AUTH_REQUIREMENT: AuthRequirement = AuthRequirement::BearerToken;
        &AUTH_REQUIREMENT
    }

    fn parse_inbound(
        &self,
        _raw_payload: &[u8],
        _auth_evidence: &ironclaw_product_adapters::ProtocolAuthEvidence,
    ) -> Result<ironclaw_product_adapters::ParsedProductInbound, ProductAdapterError> {
        Err(ProductAdapterError::Internal {
            detail: ironclaw_product_adapters::RedactedString::new("not used"),
        })
    }

    async fn render_outbound(
        &self,
        _envelope: ProductOutboundEnvelope,
        _egress: &dyn ProtocolHttpEgress,
        _delivery_sink: &dyn OutboundDeliverySink,
    ) -> Result<ProductRenderOutcome, ProductAdapterError> {
        Ok(ProductRenderOutcome::Deferred)
    }
}

struct FailingAdapter {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    error: ProductAdapterError,
}

impl FailingAdapter {
    fn new(error: ProductAdapterError) -> Self {
        Self {
            adapter_id: ProductAdapterId::new("failing_test").expect("valid adapter id"),
            installation_id: AdapterInstallationId::new("failing_install").expect("valid install"),
            error,
        }
    }
}

#[async_trait]
impl ProductAdapter for FailingAdapter {
    fn adapter_id(&self) -> &ProductAdapterId {
        &self.adapter_id
    }

    fn installation_id(&self) -> &AdapterInstallationId {
        &self.installation_id
    }

    fn surface_kind(&self) -> ProductSurfaceKind {
        ProductSurfaceKind::SynchronousApi
    }

    fn capabilities(&self) -> &ProductAdapterCapabilities {
        &SYNC_ADAPTER_CAPABILITIES
    }

    fn auth_requirement(&self) -> &AuthRequirement {
        static AUTH_REQUIREMENT: AuthRequirement = AuthRequirement::BearerToken;
        &AUTH_REQUIREMENT
    }

    fn parse_inbound(
        &self,
        _raw_payload: &[u8],
        _auth_evidence: &ironclaw_product_adapters::ProtocolAuthEvidence,
    ) -> Result<ironclaw_product_adapters::ParsedProductInbound, ProductAdapterError> {
        Err(ProductAdapterError::Internal {
            detail: RedactedString::new("not used"),
        })
    }

    async fn render_outbound(
        &self,
        _envelope: ProductOutboundEnvelope,
        _egress: &dyn ProtocolHttpEgress,
        _delivery_sink: &dyn OutboundDeliverySink,
    ) -> Result<ProductRenderOutcome, ProductAdapterError> {
        Err(self.error.clone())
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

fn actor() -> TurnActor {
    TurnActor::new(UserId::new("user-product-outbound").expect("valid user"))
}

fn telegram_adapter() -> TelegramV2Adapter {
    TelegramV2Adapter::new(TelegramV2AdapterConfig {
        adapter_id: ProductAdapterId::new("telegram_v2").expect("valid adapter id"),
        installation_id: AdapterInstallationId::new("install_alpha").expect("valid installation"),
        group_trigger_policy: GroupTriggerPolicy {
            bot_username: "ironclaw_bot".into(),
            bot_user_id: 9000,
            recognized_commands: vec!["start".into()],
        },
        egress_credential_handle: EgressCredentialHandle::new("telegram_bot_token")
            .expect("valid credential handle"),
        auth_requirement: AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
        },
        progress_push_enabled: false,
    })
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
                origin: RunNotificationOrigin::Triggered {
                    trigger: trigger_context(),
                },
            }),
        },
        turn_run_id: Some(TurnRunId::new()),
        projection_ref: ironclaw_outbound::ProjectionUpdateRef::new("projection:update:1")
            .expect("valid projection ref"),
        attempted_at: Utc::now(),
    }
}

fn requested_outbound_delivery_request(
    scope: TurnScope,
    actor: TurnActor,
    modality: CommunicationModality,
) -> ironclaw_outbound::PrepareCommunicationDeliveryRequest {
    ironclaw_outbound::PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope,
            actor,
            modality,
            intent: CommunicationDeliveryIntent::RequestedOutbound(RequestedOutboundContext {
                requested_target: validated_reply_target(),
                requested_kind: RequestedOutboundKind::ProductMessage,
            }),
        },
        turn_run_id: Some(TurnRunId::new()),
        projection_ref: ironclaw_outbound::ProjectionUpdateRef::new("projection:update:requested")
            .expect("valid projection ref"),
        attempted_at: Utc::now(),
    }
}

fn system_event_delivery_request(
    scope: TurnScope,
) -> ironclaw_outbound::PrepareCommunicationDeliveryRequest {
    ironclaw_outbound::PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope,
            actor: actor(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: RunNotificationEventKind::FinalReplyReady,
                origin: RunNotificationOrigin::SystemEvent {
                    reason: SystemEventReasonCode::Generic,
                },
            }),
        },
        turn_run_id: Some(TurnRunId::new()),
        projection_ref: ironclaw_outbound::ProjectionUpdateRef::new("projection:update:system")
            .expect("valid projection ref"),
        attempted_at: Utc::now(),
    }
}

fn trigger_context() -> ironclaw_outbound::TriggerCommunicationContext {
    ironclaw_outbound::TriggerCommunicationContext {
        trigger_origin_ref: TriggerOriginRef::new("trigger-origin:product-outbound")
            .expect("valid trigger origin ref"),
        trigger_source_kind: TriggerSourceKind::Schedule,
        fire_slot: TriggerFireSlot::new("fire-slot:product-outbound")
            .expect("valid trigger fire slot"),
    }
}

fn final_reply_payload() -> ProductOutboundPayload {
    ProductOutboundPayload::FinalReply(FinalReplyView {
        turn_run_id: TurnRunId::new(),
        text: "final reply from product workflow".into(),
        generated_at: Utc::now(),
    })
}

fn progress_payload() -> ProductOutboundPayload {
    ProductOutboundPayload::Progress(ProgressUpdateView {
        turn_run_id: TurnRunId::new(),
        kind: ProgressKind::Typing,
        generated_at: Utc::now(),
    })
}

fn configured_policy<'a>(
    store: &'a InMemoryOutboundStateStore,
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
        final_reply_target: Some(validated_reply_target()),
        progress_target: None,
        approval_prompt_target: None,
        auth_prompt_target: None,
        default_modality: Some(CommunicationModality::Text),
        updated_at: Utc::now(),
        updated_by: UserId::new("pref-updater").expect("valid updater"),
    }
}

#[tokio::test]
async fn authorized_final_reply_renders_through_telegram_egress_after_validation() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let outcome = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:1").expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect("delivery succeeds");

    let ProductOutboundDeliveryOutcome::Rendered {
        attempt,
        render_outcome,
    } = outcome
    else {
        panic!("expected rendered outcome");
    };
    assert_eq!(attempt.scope, scope);
    assert_eq!(validator.calls(), 1);
    assert_eq!(preferences.load_calls(), 1);
    assert_eq!(resolver.calls(), 1);
    assert_eq!(resolver.called_targets(), vec![validated_reply_target()]);
    assert_eq!(egress.calls().len(), 1);
    assert_eq!(egress.calls()[0].host, "api.telegram.org");
    assert_eq!(egress.calls()[0].path, "/sendMessage");
    assert_eq!(
        egress.calls()[0].credential_handle.as_deref(),
        Some("telegram_bot_token")
    );
    let body: serde_json::Value =
        serde_json::from_slice(&egress.calls()[0].body).expect("request body json");
    assert_eq!(body["chat_id"], -100);
    assert_eq!(body["text"], "final reply from product workflow");
    assert_eq!(sink.statuses().len(), 1);
    assert!(matches!(
        sink.statuses()[0],
        DeliveryStatus::Delivered { .. }
    ));
    assert!(matches!(
        render_outcome,
        ProductRenderOutcome::DeliveryRecorded
    ));
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
}

#[tokio::test]
async fn synchronous_response_marks_attempt_delivered() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = SynchronousResponseAdapter::new();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    let sink = FakeOutboundDeliverySink::new();

    let outcome = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:sync").expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect("delivery succeeds");

    let ProductOutboundDeliveryOutcome::Rendered { render_outcome, .. } = outcome else {
        panic!("expected rendered outcome");
    };
    assert!(matches!(
        render_outcome,
        ProductRenderOutcome::SynchronousResponse(_)
    ));
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
}

#[tokio::test]
async fn deferred_render_keeps_attempt_pending_and_skips_delivery_status_side_effects() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = DeferredAdapter::new();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    let sink = FakeOutboundDeliverySink::new();

    let outcome = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:deferred")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect("delivery succeeds");

    let ProductOutboundDeliveryOutcome::Rendered { render_outcome, .. } = outcome else {
        panic!("expected rendered outcome");
    };
    assert!(matches!(render_outcome, ProductRenderOutcome::Deferred));
    assert_eq!(validator.calls(), 1);
    assert_eq!(preferences.load_calls(), 1);
    assert_eq!(resolver.calls(), 1);
    assert!(egress.calls().is_empty());
    assert!(sink.statuses().is_empty());
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Pending
    );
}

#[tokio::test]
async fn status_update_failure_after_render_does_not_turn_send_into_failure() {
    let scope = scope();
    let store = StatusFailingOutboundStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    store
        .put_communication_preference(preference_record(&scope))
        .await
        .expect("seed preference");
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = OutboundPolicyService::new(&store, &ACCESS_POLICY, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let outcome = prepare_and_render_product_outbound(
        &policy,
        &store,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:status-fail")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect("render success is returned even if bookkeeping update fails");

    let ProductOutboundDeliveryOutcome::RenderedStatusUpdateFailed {
        attempt,
        render_outcome,
        status_update_error,
    } = outcome
    else {
        panic!("expected rendered outcome with status update failure");
    };
    assert!(matches!(
        render_outcome,
        ProductRenderOutcome::DeliveryRecorded
    ));
    assert_eq!(
        status_update_error,
        ProductOutboundStatusUpdateFailure::Backend
    );
    let status_update_requests = store.status_update_requests();
    assert_eq!(status_update_requests.len(), 1);
    assert_eq!(status_update_requests[0].delivery_id, attempt.delivery_id);
    assert_eq!(status_update_requests[0].scope, attempt.scope);
    assert_eq!(
        status_update_requests[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
    assert_eq!(status_update_requests[0].failure_kind, None);
    assert_eq!(egress.calls().len(), 1);
    assert!(matches!(
        sink.statuses().as_slice(),
        [DeliveryStatus::Delivered { .. }]
    ));
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Pending
    );
}

#[tokio::test]
async fn requested_outbound_preserves_actor_and_modality_before_rendering() {
    let scope = scope();
    let requesting_actor = actor();
    let requested_modality = CommunicationModality::Voice;
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    validator.require_actor(requesting_actor.clone());
    validator.require_modality(requested_modality);
    let preferences = FakePreferenceRepository::default();
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let outcome = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: requested_outbound_delivery_request(
                scope.clone(),
                requesting_actor,
                requested_modality,
            ),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:requested")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect("delivery succeeds");

    assert!(matches!(
        outcome,
        ProductOutboundDeliveryOutcome::Rendered { .. }
    ));
    assert_eq!(
        preferences.load_calls(),
        0,
        "requested outbound uses the explicit target instead of preferences"
    );
    assert_eq!(validator.calls(), 1);
    assert_eq!(resolver.calls(), 1);
    assert_eq!(resolver.called_targets(), vec![validated_reply_target()]);
    assert_eq!(egress.calls().len(), 1);
    assert_eq!(sink.statuses().len(), 1);
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
}

#[tokio::test]
async fn mismatched_payload_kind_marks_authorized_attempt_failed_without_render() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let err = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: progress_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:mismatch")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect_err("payload kind mismatch fails before render");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductOutboundDeliveryError::PayloadKindMismatch {
            status_update_error: None,
            ..
        }
    ));
    assert_eq!(preferences.load_calls(), 1);
    assert_eq!(validator.calls(), 1);
    assert_eq!(resolver.calls(), 0);
    assert!(egress.calls().is_empty());
    assert!(sink.statuses().is_empty());
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
async fn payload_kind_mismatch_preserves_status_update_failure() {
    let scope = scope();
    let store = StatusFailingOutboundStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    store
        .put_communication_preference(preference_record(&scope))
        .await
        .expect("seed preference");
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = OutboundPolicyService::new(&store, &ACCESS_POLICY, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let err = prepare_and_render_product_outbound(
        &policy,
        &store,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: progress_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:mismatch-status-fail")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect_err("payload kind mismatch preserves status update failure");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductOutboundDeliveryError::PayloadKindMismatch {
            status_update_error: Some(ProductOutboundStatusUpdateFailure::Backend),
            ..
        }
    ));
    assert_eq!(validator.calls(), 1);
    assert_eq!(resolver.calls(), 0);
    assert!(egress.calls().is_empty());
    assert!(sink.statuses().is_empty());
    let attempts = store.list_delivery_attempts(scope.clone()).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Pending
    );
    let status_update_requests = store.status_update_requests();
    assert_eq!(status_update_requests.len(), 1);
    assert_eq!(
        status_update_requests[0].delivery_id,
        attempts[0].delivery_id
    );
    assert_eq!(status_update_requests[0].scope, scope);
    assert_eq!(
        status_update_requests[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed
    );
    assert_eq!(
        status_update_requests[0].failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::Rejected)
    );
}

#[tokio::test]
async fn target_metadata_failure_with_status_update_failure_preserves_workflow_error() {
    let scope = scope();
    let store = StatusFailingOutboundStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    store
        .put_communication_preference(preference_record(&scope))
        .await
        .expect("seed preference");
    let resolver = FakeProductOutboundTargetResolver::default();
    resolver.fail();
    let policy = OutboundPolicyService::new(&store, &ACCESS_POLICY, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let err = prepare_and_render_product_outbound(
        &policy,
        &store,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:target-fail-status-update")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect_err("target metadata failure propagates");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductOutboundDeliveryError::Workflow {
            status_update_error: Some(ProductOutboundStatusUpdateFailure::Backend),
            ..
        }
    ));
    assert_eq!(validator.calls(), 1);
    assert_eq!(resolver.calls(), 1);
    assert!(egress.calls().is_empty());
    assert!(sink.statuses().is_empty());
    let attempts = store.list_delivery_attempts(scope.clone()).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Pending
    );
    let status_update_requests = store.status_update_requests();
    assert_eq!(status_update_requests.len(), 1);
    assert_eq!(
        status_update_requests[0].delivery_id,
        attempts[0].delivery_id
    );
    assert_eq!(status_update_requests[0].scope, scope);
    assert_eq!(
        status_update_requests[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed
    );
    assert_eq!(
        status_update_requests[0].failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::TransportUnavailable)
    );
}

#[tokio::test]
async fn target_metadata_failure_marks_attempt_failed_without_render() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    resolver.fail();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let err = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:target-fail")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect_err("target metadata failure propagates");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductOutboundDeliveryError::Workflow {
            status_update_error: None,
            ..
        }
    ));
    assert_eq!(validator.calls(), 1);
    assert_eq!(resolver.calls(), 1);
    assert!(egress.calls().is_empty());
    assert!(sink.statuses().is_empty());
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
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
async fn target_metadata_rejection_errors_mark_attempt_failed_rejected() {
    let cases = [
        ProductWorkflowError::BindingAccessDenied,
        ProductWorkflowError::BindingRequired {
            reason: "actor binding required".into(),
        },
        ProductWorkflowError::UnknownInstallation,
        ProductWorkflowError::InvalidBindingRequest {
            reason: "invalid binding".into(),
        },
    ];

    for (index, workflow_error) in cases.into_iter().enumerate() {
        let scope = scope();
        let store = InMemoryOutboundStateStore::default();
        let validator = FakeReplyTargetBindingValidator::default();
        validator.allow(validated_reply_target());
        let preferences = FakePreferenceRepository::default();
        seed_preference(&preferences, &scope);
        let resolver = FakeProductOutboundTargetResolver::default();
        resolver.fail_with(workflow_error);
        let policy = configured_policy(&store, &validator);
        let adapter = telegram_adapter();
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        egress.allow_credential_handle("telegram_bot_token");
        let sink = FakeOutboundDeliverySink::new();

        let err = prepare_and_render_product_outbound(
            &policy,
            &preferences,
            &resolver,
            ProductOutboundDeliveryRequest {
                delivery: delivery_request(scope.clone()),
                payload: final_reply_payload(),
                projection_cursor: ProjectionCursor::new(format!(
                    "cursor:outbound:workflow-rejected-{index}"
                ))
                .expect("valid cursor"),
                adapter: &adapter,
                egress: &egress,
                delivery_sink: &sink,
                require_direct_message_target: false,
            },
        )
        .await
        .expect_err("target metadata rejection propagates");

        assert!(matches!(
            err,
            ironclaw_product_workflow::ProductOutboundDeliveryError::Workflow {
                status_update_error: None,
                ..
            }
        ));
        assert_eq!(validator.calls(), 1);
        assert_eq!(resolver.calls(), 1);
        assert!(egress.calls().is_empty());
        assert!(sink.statuses().is_empty());
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
}

#[tokio::test]
async fn keep_alive_payload_marks_authorized_attempt_failed_without_render() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let requesting_actor = actor();
    let err = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: requested_outbound_delivery_request(
                scope.clone(),
                requesting_actor,
                CommunicationModality::Text,
            ),
            payload: ProductOutboundPayload::KeepAlive,
            projection_cursor: ProjectionCursor::new("cursor:outbound:keepalive")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect_err("keepalive payload is not renderable for a sendable delivery");
    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductOutboundDeliveryError::PayloadKindMismatch {
            payload_kind: None,
            status_update_error: None,
            ..
        }
    ));
    assert_eq!(validator.calls(), 1);
    assert_eq!(resolver.calls(), 0);
    assert!(egress.calls().is_empty());
    assert!(sink.statuses().is_empty());
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
async fn adapter_render_failure_is_returned_and_marks_attempt_failed() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    egress.program_response(
        "api.telegram.org",
        Ok(EgressResponse::new(500, br#"{"ok":false}"#.to_vec())),
    );
    let sink = FakeOutboundDeliverySink::new();

    let err = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:adapter-fail")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect_err("adapter failure propagates");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductOutboundDeliveryError::Adapter {
            status_update_error: None,
            ..
        }
    ));
    assert_eq!(validator.calls(), 1);
    assert_eq!(resolver.calls(), 1);
    assert_eq!(egress.calls().len(), 1);
    let statuses = sink.statuses();
    assert_eq!(statuses.len(), 1);
    assert!(matches!(
        statuses[0],
        DeliveryStatus::FailedRetryable { .. }
    ));
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
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
async fn adapter_non_retryable_errors_mark_attempt_failed_rejected() {
    let cases = [
        ProductAdapterError::EgressDenied {
            reason: RedactedString::new("policy denied"),
        },
        ProductAdapterError::EgressUndeclaredHost {
            host: "api.example.invalid".into(),
        },
        ProductAdapterError::InvalidIdentifier {
            kind: "chat",
            reason: "not canonical".into(),
        },
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::InvalidRequest,
            status_code: 400,
            retryable: false,
            reason: RedactedString::new("not retryable"),
        },
    ];

    for (index, adapter_error) in cases.into_iter().enumerate() {
        let scope = scope();
        let store = InMemoryOutboundStateStore::default();
        let validator = FakeReplyTargetBindingValidator::default();
        validator.allow(validated_reply_target());
        let preferences = FakePreferenceRepository::default();
        seed_preference(&preferences, &scope);
        let resolver = FakeProductOutboundTargetResolver::default();
        let policy = configured_policy(&store, &validator);
        let adapter = FailingAdapter::new(adapter_error);
        let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
        let sink = FakeOutboundDeliverySink::new();

        let err = prepare_and_render_product_outbound(
            &policy,
            &preferences,
            &resolver,
            ProductOutboundDeliveryRequest {
                delivery: delivery_request(scope.clone()),
                payload: final_reply_payload(),
                projection_cursor: ProjectionCursor::new(format!(
                    "cursor:outbound:adapter-rejected-{index}"
                ))
                .expect("valid cursor"),
                adapter: &adapter,
                egress: &egress,
                delivery_sink: &sink,
                require_direct_message_target: false,
            },
        )
        .await
        .expect_err("adapter rejection propagates");

        assert!(matches!(
            err,
            ironclaw_product_workflow::ProductOutboundDeliveryError::Adapter {
                status_update_error: None,
                ..
            }
        ));
        assert_eq!(validator.calls(), 1);
        assert_eq!(resolver.calls(), 1);
        assert!(egress.calls().is_empty());
        assert!(sink.statuses().is_empty());
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
}

#[tokio::test]
async fn adapter_render_failure_preserves_adapter_error_when_status_update_fails() {
    let scope = scope();
    let store = StatusFailingOutboundStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    store
        .put_communication_preference(preference_record(&scope))
        .await
        .expect("seed preference");
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = OutboundPolicyService::new(&store, &ACCESS_POLICY, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    egress.program_response(
        "api.telegram.org",
        Ok(EgressResponse::new(500, br#"{"ok":false}"#.to_vec())),
    );
    let sink = FakeOutboundDeliverySink::new();

    let err = prepare_and_render_product_outbound(
        &policy,
        &store,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:adapter-status-fail")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect_err("adapter failure is primary even if status update fails");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductOutboundDeliveryError::Adapter {
            status_update_error: Some(ProductOutboundStatusUpdateFailure::Backend),
            ..
        }
    ));
    let attempts = store.list_delivery_attempts(scope.clone()).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Pending
    );
    let status_update_requests = store.status_update_requests();
    assert_eq!(status_update_requests.len(), 1);
    assert_eq!(
        status_update_requests[0].delivery_id,
        attempts[0].delivery_id
    );
    assert_eq!(status_update_requests[0].scope, scope);
    assert_eq!(
        status_update_requests[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed
    );
    assert_eq!(
        status_update_requests[0].failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::TransportUnavailable)
    );
}

#[tokio::test]
async fn revoked_or_rejected_target_does_not_call_render_or_egress() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let outcome = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:2").expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect("delivery outcome");

    let ProductOutboundDeliveryOutcome::Rejected { attempt } = outcome else {
        panic!("expected rejected outcome");
    };
    assert_eq!(attempt.scope, scope);
    assert_eq!(
        attempt.failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::AuthorizationRevoked)
    );
    assert_eq!(validator.calls(), 1);
    assert_eq!(preferences.load_calls(), 1);
    assert_eq!(resolver.calls(), 0);
    assert!(egress.calls().is_empty());
    assert!(sink.statuses().is_empty());
    assert_eq!(store.list_delivery_attempts(scope).await.unwrap().len(), 1);
}

#[tokio::test]
async fn no_delivery_system_event_does_not_call_render_or_egress() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let preferences = FakePreferenceRepository::default();
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let outcome = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: system_event_delivery_request(scope.clone()),
            payload: progress_payload(),
            projection_cursor: ProjectionCursor::new("cursor:outbound:3").expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect("no delivery is still success");

    assert!(matches!(
        outcome,
        ProductOutboundDeliveryOutcome::NoDelivery
    ));
    assert_eq!(validator.calls(), 0);
    assert_eq!(preferences.load_calls(), 0);
    assert_eq!(resolver.calls(), 0);
    assert!(egress.calls().is_empty());
    assert!(sink.statuses().is_empty());
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .unwrap()
            .is_empty()
    );
}

// ── require_direct_message_target flag threading ──────────────────────────────
//
// Verify that the flag is forwarded to the resolver and that a resolver
// returning `OutboundTargetNotDirectMessage` maps to `Rejected` in the audit
// trail (Fix 1 + Fix 5 contract).

#[tokio::test]
async fn require_direct_message_true_propagates_to_resolver_and_maps_to_rejected() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = DmRequiringFakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    let sink = FakeOutboundDeliverySink::new();

    let err = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:dm-required-true")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: true,
        },
    )
    .await
    .expect_err("require_direct_message=true with non-DM resolver must fail");

    // The error must be Workflow { source: OutboundTargetNotDirectMessage }.
    assert!(
        matches!(
            err,
            ironclaw_product_workflow::ProductOutboundDeliveryError::Workflow {
                source: ProductWorkflowError::OutboundTargetNotDirectMessage,
                status_update_error: None,
                ..
            }
        ),
        "unexpected error: {err:?}"
    );
    // The flag must have been forwarded to the resolver.
    let calls = resolver.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].1,
        "require_direct_message must be true at resolver"
    );
    // Audit trail must record Rejected (not Unknown).
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].failure_kind,
        Some(ironclaw_outbound::DeliveryFailureKind::Rejected),
        "OutboundTargetNotDirectMessage must map to Rejected, not Unknown"
    );
}

#[tokio::test]
async fn require_direct_message_false_does_not_trigger_dm_rejection() {
    let scope = scope();
    let store = InMemoryOutboundStateStore::default();
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = DmRequiringFakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = telegram_adapter();
    let egress = FakeProtocolHttpEgress::new(["api.telegram.org".to_string()]);
    egress.allow_credential_handle("telegram_bot_token");
    let sink = FakeOutboundDeliverySink::new();

    let outcome = prepare_and_render_product_outbound(
        &policy,
        &preferences,
        &resolver,
        ProductOutboundDeliveryRequest {
            delivery: delivery_request(scope.clone()),
            payload: final_reply_payload(),
            projection_cursor: ProjectionCursor::new("cursor:dm-required-false")
                .expect("valid cursor"),
            adapter: &adapter,
            egress: &egress,
            delivery_sink: &sink,
            require_direct_message_target: false,
        },
    )
    .await
    .expect("require_direct_message=false must not trigger DM rejection");

    assert!(
        matches!(outcome, ProductOutboundDeliveryOutcome::Rendered { .. }),
        "unexpected outcome: {outcome:?}"
    );
    // Flag must have been forwarded as false.
    let calls = resolver.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        !calls[0].1,
        "require_direct_message must be false at resolver"
    );
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
}

// ---------------------------------------------------------------------------
// Delivery coordinator (extension-runtime §5.4; OUT-1..7, ING-11)
// ---------------------------------------------------------------------------

use std::collections::VecDeque;
use std::sync::Arc;

use ironclaw_product_adapters::{
    ChannelAdapter, ChannelError, DeliveryReport, InboundOutcome, OutboundEnvelope,
    PartDeliveryOutcome, VerifiedInbound,
};
use ironclaw_product_workflow::{
    ChannelDeliveryResolver, CoordinatedDeliveryError, CoordinatedDeliveryOutcome,
    CoordinatedDeliveryRequest, DeliveryCoordinator, DeliveryIntent, DeliveryReplyContextSource,
    DeliveryRetryPolicy, ResolvedChannelDelivery,
};

struct CoordinatorDenyAllEgress;

#[async_trait]
impl ironclaw_host_api::RestrictedEgress for CoordinatorDenyAllEgress {
    async fn send(
        &self,
        _request: ironclaw_host_api::RestrictedEgressRequest,
    ) -> Result<ironclaw_host_api::RestrictedEgressResponse, ironclaw_host_api::RestrictedEgressError>
    {
        Err(ironclaw_host_api::RestrictedEgressError::PolicyDenied)
    }
}

/// Scripted channel adapter: pops one report per deliver call, records the
/// envelope it saw, and captures the durable attempt status AT deliver time
/// (proving OUT-3: `Sending` is persisted before any vendor work).
struct ScriptedChannelAdapter {
    reports: Mutex<VecDeque<Result<DeliveryReport, ChannelError>>>,
    envelopes: Mutex<Vec<OutboundEnvelope>>,
    observed_status: Mutex<Vec<ironclaw_outbound::OutboundDeliveryStatus>>,
    store: Arc<InMemoryOutboundStateStore>,
    scope: TurnScope,
}

impl ScriptedChannelAdapter {
    fn new(
        store: Arc<InMemoryOutboundStateStore>,
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
        _egress: &dyn ironclaw_host_api::RestrictedEgress,
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
            extension_id: extension_id.to_string(),
            installation_id: "inst-1".to_string(),
            adapter: Arc::clone(&self.adapter) as Arc<dyn ChannelAdapter>,
            egress: Arc::new(CoordinatorDenyAllEgress),
        })
    }
}

struct FixedReplyContext(Vec<u8>);

#[async_trait]
impl DeliveryReplyContextSource for FixedReplyContext {
    async fn reply_context(
        &self,
        _extension_id: &str,
        _installation_id: &str,
        _conversation_fingerprint: &str,
    ) -> Option<Vec<u8>> {
        Some(self.0.clone())
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
    store: &Arc<InMemoryOutboundStateStore>,
    adapter: &Arc<ScriptedChannelAdapter>,
) -> DeliveryCoordinator {
    DeliveryCoordinator::new(
        Arc::clone(store) as Arc<dyn ironclaw_outbound::OutboundStateStore>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(adapter),
            unavailable: false,
        }),
        Arc::new(FixedReplyContext(b"vendor-reply-ctx".to_vec())),
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    )
}

fn coordinated_final_reply(scope: TurnScope, extension_id: &str) -> CoordinatedDeliveryRequest<'_> {
    CoordinatedDeliveryRequest {
        intent: DeliveryIntent::FinalReply,
        delivery: delivery_request(scope),
        parts: vec![ironclaw_product_adapters::OutboundPart::Text(
            "final reply".to_string(),
        )],
        thread_anchor: Some("thread-1".to_string()),
        require_direct_message_target: false,
        extension_id,
    }
}

#[tokio::test]
async fn coordinator_persists_sending_before_the_adapter_delivers() {
    let scope = scope();
    let store = Arc::new(InMemoryOutboundStateStore::default());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            parts: vec![sent("ts-100")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let outcome = coordinator
        .deliver(
            &policy,
            &preferences,
            &resolver,
            coordinated_final_reply(scope.clone(), "vendorx"),
        )
        .await
        .expect("delivery drives");

    let CoordinatedDeliveryOutcome::Delivered {
        attempt,
        vendor_message_refs,
    } = outcome
    else {
        panic!("expected delivered outcome");
    };
    assert_eq!(vendor_message_refs, vec!["ts-100".to_string()]);
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
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
}

#[tokio::test]
async fn coordinator_retries_fully_retryable_reports_then_delivers() {
    let scope = scope();
    let store = Arc::new(InMemoryOutboundStateStore::default());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
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
            &preferences,
            &resolver,
            coordinated_final_reply(scope.clone(), "vendorx"),
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
    let store = Arc::new(InMemoryOutboundStateStore::default());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
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
            &preferences,
            &resolver,
            coordinated_final_reply(scope.clone(), "vendorx"),
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
async fn coordinator_recovery_marks_interrupted_sending_attempts_unknown() {
    let scope = scope();
    let store = Arc::new(InMemoryOutboundStateStore::default());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
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
            &preferences,
            &resolver,
            coordinated_final_reply(scope.clone(), "vendorx"),
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
async fn coordinator_rejects_new_deliveries_while_draining() {
    let scope = scope();
    let store = Arc::new(InMemoryOutboundStateStore::default());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = coordinator_over(&store, &adapter);
    coordinator.begin_drain();

    let error = coordinator
        .deliver(
            &policy,
            &preferences,
            &resolver,
            coordinated_final_reply(scope.clone(), "vendorx"),
        )
        .await
        .expect_err("draining rejects new work");
    assert!(matches!(error, CoordinatedDeliveryError::Draining));
    assert_eq!(adapter.deliver_calls(), 0);
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert!(attempts.is_empty(), "no attempt recorded while draining");
}

#[tokio::test]
async fn coordinator_fails_closed_when_the_channel_is_unavailable() {
    let scope = scope();
    let store = Arc::new(InMemoryOutboundStateStore::default());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver::default();
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn ironclaw_outbound::OutboundStateStore>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: true,
        }),
        Arc::new(FixedReplyContext(Vec::new())),
        DeliveryRetryPolicy::default(),
    );

    let error = coordinator
        .deliver(
            &policy,
            &preferences,
            &resolver,
            coordinated_final_reply(scope.clone(), "vendorx"),
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
