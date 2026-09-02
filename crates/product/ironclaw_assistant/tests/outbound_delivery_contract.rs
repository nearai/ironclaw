// arch-exempt: large_file, mechanical OutboundStateStore<ironclaw_filesystem::InMemoryBackend> -> OutboundStateStore<InMemoryBackend> §4.3 store consolidation, no logic change, plan #6168
use ironclaw_extension_contracts::channel_adapter::ChannelDelivery;
use ironclaw_extension_contracts::reply::ReplySink;
use ironclaw_extension_contracts::test_support::fakes::RecordingReplySink;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_assistant::{
    ProductOutboundTargetResolver, ProductSurfaceFailure, VerifiedProductOutboundTargetMetadata,
};
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
    OutboundPolicyService, OutboundPushCandidate, OutboundStateStore, OutboundStateStorePort,
    ReplyTargetBindingClaim, ReplyTargetBindingValidator, RunNotificationContext,
    RunNotificationEventKind, RunNotificationOrigin, ThreadProjectionAccessClaim,
    ThreadProjectionAccessPolicy, ThreadProjectionAccessRequest,
    VersionedCommunicationPreferenceRecord, WriteCommunicationPreferenceRequest,
};
use ironclaw_threads::ThreadScope;
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
                event_kind: RunNotificationEventKind::ApprovalNeeded,
                origin: RunNotificationOrigin::LiveSourceRoute {
                    source_route: ironclaw_outbound::SourceRouteContext {
                        reply_target_binding_ref: validated_reply_target(),
                    },
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
use ironclaw_extension_contracts::channel_adapter::{
    ChannelError, DeliveryReport, OutboundEnvelope, OutboundVisibility, PartDeliveryOutcome,
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
    delivery_sends: std::sync::atomic::AtomicUsize,
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
            delivery_sends: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn delivery_sends(&self) -> usize {
        self.delivery_sends
            .load(std::sync::atomic::Ordering::SeqCst)
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

    async fn send_scripted(
        &self,
        envelope: OutboundEnvelope,
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
            .unwrap_or(Err(ChannelError::Unsupported))
    }
}

#[async_trait]
impl ChannelDelivery for ScriptedChannelAdapter {
    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        _egress: &dyn ironclaw_extension_contracts::tool_adapter::RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        self.delivery_sends
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.send_scripted(envelope).await
    }
}

struct StaticChannelResolver {
    adapter: Arc<ScriptedChannelAdapter>,
    unavailable: bool,
    reply_transport: Option<ironclaw_extension_contracts::channel::ReplyTransport>,
    requires_enrollment: bool,
}

impl ChannelDeliveryResolver for StaticChannelResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        if self.unavailable {
            return None;
        }
        // A declared reply transport binds a reply sink, exactly as
        // activation guarantees. The coordinator never drives it: a run's
        // answer is reconciled by reply publication, so any sink recorded
        // here proves the coordinator stayed off it.
        let reply = self
            .reply_transport
            .map(|_| Arc::new(RecordingReplySink::default()) as Arc<dyn ReplySink>);
        Some(ResolvedChannelDelivery {
            extension_id: ExtensionId::new(extension_id).expect("valid extension id"),
            installation_id: AdapterInstallationId::new("inst-1").expect("valid installation id"),
            reply,
            delivery: Some(Arc::clone(&self.adapter) as Arc<dyn ChannelDelivery>),
            egress: Arc::new(CoordinatorDenyAllEgress),
            reply_transport: self.reply_transport,
            requires_enrollment: self.requires_enrollment,
            declared_egress_hosts: Vec::new(),
            generation: 0,
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
    ) -> Result<Option<Vec<u8>>, ironclaw_product_contracts::delivery::DeliveryReplyContextError>
    {
        self.asked.lock().expect("lock").push((
            extension_id.as_str().to_string(),
            installation_id.as_str().to_string(),
        ));
        Ok(Some(self.bytes.clone()))
    }
}

struct FailingReplyContext;

#[async_trait]
impl DeliveryReplyContextSource for FailingReplyContext {
    async fn reply_context(
        &self,
        _extension_id: &ExtensionId,
        _installation_id: &AdapterInstallationId,
        _conversation_fingerprint: &str,
    ) -> Result<Option<Vec<u8>>, ironclaw_product_contracts::delivery::DeliveryReplyContextError>
    {
        Err(ironclaw_product_contracts::delivery::DeliveryReplyContextError)
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
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Message),
            requires_enrollment: false,
        }),
        Arc::clone(&reply_context) as Arc<dyn DeliveryReplyContextSource>,
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
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

/// The policy-class send the live source route still drives through the
/// coordinator: a gate prompt on the originating conversation. (A run's
/// answer is not one of these — reply publication owns it.)
fn coordinated_gate_prompt<'a>(
    scope: TurnScope,
    extension_id: &'a str,
    thread_scope: &'a ThreadScope,
) -> CoordinatedDeliveryRequest<'a> {
    CoordinatedDeliveryRequest {
        intent: DeliveryIntent::GatePrompt,
        delivery: delivery_request(scope),
        parts: vec![
            ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
                "gate prompt".to_string(),
            ),
        ],
        thread_anchor: Some("thread-1".to_string()),
        require_direct_message_target: false,
        extension_id,
        thread_scope,
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-100")],
        })],
    ));
    let (coordinator, reply_context) = coordinator_over_recording_reply_lookups(&store, &adapter);

    let thread_scope = project_thread_scope();
    let request = coordinated_gate_prompt(scope.clone(), "vendorx", &thread_scope);
    let outcome = coordinator
        .deliver(&policy, &resolver, request)
        .await
        .expect("delivery drives");

    let CoordinatedDeliveryOutcome::Delivered {
        attempt: _,
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-dm")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let thread_scope = project_thread_scope();
    let request = CoordinatedDeliveryRequest {
        intent: DeliveryIntent::GatePrompt,
        delivery: delivery_request(scope.clone()),
        parts: vec![
            ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
                "dm only".to_string(),
            ),
        ],
        thread_anchor: Some("thread-1".to_string()),
        require_direct_message_target: true,
        extension_id: "vendorx",
        thread_scope: &thread_scope,
    };
    let error = coordinator
        .deliver(&policy, &resolver, request)
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-should-not-happen")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_gate_prompt(scope.clone(), "vendorx", &project_thread_scope()),
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
                prune_registrations: Vec::new(),
                parts: vec![retryable_part()],
            }),
            Ok(DeliveryReport {
                prune_registrations: Vec::new(),
                parts: vec![sent("ts-200")],
            }),
        ],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_gate_prompt(scope.clone(), "vendorx", &project_thread_scope()),
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-300"), retryable_part()],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);

    let thread_scope = project_thread_scope();
    let mut request = coordinated_gate_prompt(scope.clone(), "vendorx", &thread_scope);
    request.parts.push(
        ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
            "second part".to_string(),
        ),
    );
    let outcome = coordinator
        .deliver(&policy, &resolver, request)
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
async fn malformed_adapter_report_cardinality_is_terminal_unknown_without_retry() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-only-one-proof")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);
    let thread_scope = project_thread_scope();
    let mut request = coordinated_gate_prompt(scope.clone(), "vendorx", &thread_scope);
    request.parts.push(
        ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
            "second part".to_string(),
        ),
    );

    let outcome = coordinator
        .deliver(&policy, &FakeProductOutboundTargetResolver, request)
        .await
        .expect("malformed report settles without retry");

    assert!(matches!(
        outcome,
        CoordinatedDeliveryOutcome::Failed {
            failure_kind: ironclaw_outbound::DeliveryFailureKind::Unknown,
            ..
        }
    ));
    assert_eq!(adapter.deliver_calls(), 1, "ambiguous egress cannot retry");
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Unknown
    );
}

#[tokio::test]
async fn chunked_adapter_report_with_more_outcomes_than_parts_is_delivered() {
    // Slack and Telegram fan one long text part out as several vendor chunks
    // and push one PartDeliveryOutcome per chunk (the adapter conformance
    // suite legalizes outcomes >= parts). Regression: the coordinator
    // required exact equality, settling fully delivered chunked replies as
    // Unknown/Failed — a model-visible failure inviting a duplicate resend of
    // a message every recipient already received.
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-chunk-1"), sent("ts-chunk-2")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);
    let thread_scope = project_thread_scope();
    let mut request = coordinated_gate_prompt(scope.clone(), "vendorx", &thread_scope);
    request.parts = vec![
        ironclaw_extension_contracts::channel_adapter::OutboundPart::Text(
            "one long prompt that the vendor splits into two chunks".to_string(),
        ),
    ];
    let result = coordinator.deliver(&policy, &resolver, request).await;

    let outcome = result.expect("chunked send delivers");
    match outcome {
        CoordinatedDeliveryOutcome::Delivered {
            vendor_message_refs,
            ..
        } => assert_eq!(
            vendor_message_refs,
            vec!["ts-chunk-1".to_string(), "ts-chunk-2".to_string()],
            "every chunk's provider evidence rides the outcome"
        ),
        other => panic!("chunked send must settle Delivered, got {other:?}"),
    }
    assert_eq!(adapter.deliver_calls(), 1);
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
}

#[tokio::test]
async fn reply_context_storage_failure_stops_before_adapter_egress() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let policy = configured_policy(&store, &validator);
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: false,
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Message),
            requires_enrollment: false,
        }),
        Arc::new(FailingReplyContext),
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: std::time::Duration::ZERO,
        },
    );

    let error = coordinator
        .deliver(
            &policy,
            &FakeProductOutboundTargetResolver,
            coordinated_gate_prompt(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect_err("reply context storage failure must fail closed");

    assert!(matches!(
        error,
        CoordinatedDeliveryError::ReplyContextUnavailable
    ));
    assert_eq!(adapter.deliver_calls(), 0, "adapter must not be called");
    let attempts = store
        .list_delivery_attempts(scope)
        .await
        .expect("list attempts");
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed,
    );
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
    let mut request = coordinated_gate_prompt(scope.clone(), "vendorx", &thread_scope);
    request.parts.push(
        ironclaw_extension_contracts::channel_adapter::OutboundPart::File(WorkspaceFile {
            path: ScopedPath::new("/workspace/untrusted.bin").expect("scoped path"),
            filename: Some("untrusted.bin".to_string()),
            mime_type: "application/octet-stream".to_string(),
            bytes: vec![0; 1],
        }),
    );

    let error = coordinator
        .deliver(&policy, &resolver, request)
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-777")],
        })],
    ));
    let reply_context = Arc::new(FixedReplyContext::new(b"vendor-reply-ctx".to_vec()));
    let coordinator = DeliveryCoordinator::new(
        Arc::new(TerminalDeliveredWriteFailingStore {
            inner: Arc::clone(&store),
            complete_before_recovery: false,
        }) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: false,
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Message),
            requires_enrollment: false,
        }),
        reply_context as Arc<dyn DeliveryReplyContextSource>,
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    );

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_gate_prompt(scope.clone(), "vendorx", &project_thread_scope()),
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-400")],
        })],
    ));
    let coordinator = coordinator_over(&store, &adapter);
    coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_gate_prompt(scope.clone(), "vendorx", &project_thread_scope()),
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
async fn coordinator_recovery_never_overwrites_a_concurrently_delivered_attempt() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let attempt = OutboundDeliveryAttempt {
        delivery_id: ironclaw_outbound::OutboundDeliveryId::new(),
        scope: scope.clone(),
        candidate: OutboundPushCandidate {
            tenant_id: scope.tenant_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id: scope.thread_id.clone(),
            turn_run_id: None,
            target: validated_reply_target(),
            kind: ironclaw_outbound::OutboundPushKind::DeliveryStatus,
            projection_ref: ironclaw_outbound::ProjectionUpdateRef::new("projection:recovery-race")
                .expect("projection ref"),
            requires_reply_target_revalidation: false,
        },
        status: ironclaw_outbound::OutboundDeliveryStatus::Sending,
        attempted_at: Utc::now(),
        failure_kind: None,
    };
    store
        .record_delivery_attempt(attempt.clone())
        .await
        .expect("seed interrupted attempt");

    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        Vec::new(),
    ));
    let coordinator = DeliveryCoordinator::new(
        Arc::new(TerminalDeliveredWriteFailingStore {
            inner: Arc::clone(&store),
            complete_before_recovery: true,
        }) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter,
            unavailable: false,
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Message),
            requires_enrollment: false,
        }),
        Arc::new(FixedReplyContext::new(Vec::new())) as Arc<dyn DeliveryReplyContextSource>,
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: std::time::Duration::ZERO,
        },
    );

    assert_eq!(
        coordinator
            .recover_interrupted_deliveries(scope.clone())
            .await
            .expect("recovery scans"),
        0,
        "a terminal row won by the send worker is not counted as recovered"
    );
    let persisted = store
        .load_delivery_attempt(scope, attempt.delivery_id)
        .await
        .expect("attempt loads")
        .expect("attempt exists");
    assert_eq!(
        persisted.status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered,
        "stale recovery must not overwrite the concurrently committed result"
    );
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
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Message),
            requires_enrollment: false,
        }),
        Arc::new(FixedReplyContext::new(Vec::new())),
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy::default(),
    );

    let error = coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_gate_prompt(scope.clone(), "vendorx", &project_thread_scope()),
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
        visibility: OutboundVisibility::Public,
    }
}

/// Regression pin (progressive reply publication): `deliver_notice` is
/// transport-blind. A notice-class intent — command feedback, a connection
/// status, a failure notice, a reaction — is a discrete message on the
/// channel's delivery half whatever the channel's reply cadence, so a
/// `stream` channel receives every one of them exactly like a `message`
/// channel does. Which notices a progressive channel still gets is the
/// observer's decision (its working indicator lives inside the published
/// reply document), never a routing rule here. The policy-class notification
/// path is pinned in the same breath so the two can never drift apart: a
/// background-run notice must still reach a user whose tab is closed.
#[tokio::test]
async fn a_stream_channel_still_receives_source_routed_notices_and_notifications() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        (0..8)
            .map(|index| {
                Ok(DeliveryReport {
                    prune_registrations: Vec::new(),
                    parts: vec![sent(&format!("ts-notice-{index}"))],
                })
            })
            .collect(),
    ));
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: false,
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Stream),
            requires_enrollment: false,
        }),
        Arc::new(FixedReplyContext::new(b"vendor-reply-ctx".to_vec()))
            as Arc<dyn DeliveryReplyContextSource>,
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    );

    // The coordinator is transport-blind: a notice a caller asks it to post
    // (command feedback, a connection status, a failure notice) is a
    // discrete message on the channel's delivery half whatever the channel's
    // reply cadence. Which notices a progressive channel still gets is the
    // observer's call — the working indicator, for one, lives inside the
    // published document there.
    for intent in [
        DeliveryIntent::Working,
        DeliveryIntent::Cleanup,
        DeliveryIntent::Reaction,
        DeliveryIntent::FailureNotice,
        DeliveryIntent::ConnectionStatus,
        DeliveryIntent::CommandFeedback,
        DeliveryIntent::ConnectRequired,
    ] {
        let mut request = working_notice(scope.clone(), "vendorx");
        request.intent = intent;
        let outcome = coordinator
            .deliver_notice(request)
            .await
            .expect("a notice on a stream channel delivers like any other");
        assert!(
            matches!(outcome, CoordinatedDeliveryOutcome::Delivered { .. }),
            "{intent:?} rides the delivery half on a stream channel too, got {outcome:?}"
        );
    }
    assert_eq!(
        adapter.deliver_calls(),
        7,
        "every source-routed notice reached the adapter's delivery half"
    );

    // ...while the policy-class notification path still delivers, so a user
    // with the tab closed keeps receiving background-run notices.
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let thread_scope = project_thread_scope();
    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_notification(scope.clone(), "vendorx", &thread_scope),
        )
        .await
        .expect("notification-routed send drives");
    assert!(
        matches!(outcome, CoordinatedDeliveryOutcome::Delivered { .. }),
        "a notification-routed send must still reach a streaming channel, got {outcome:?}"
    );
    assert_eq!(
        adapter.delivery_sends(),
        8,
        "and it rides the same delivery half"
    );
}

#[tokio::test]
async fn coordinator_notice_is_source_routed_and_persists_before_egress() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            prune_registrations: Vec::new(),
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

    for intent in [DeliveryIntent::GatePrompt, DeliveryIntent::ModelDelivery] {
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
    let mut request = coordinated_gate_prompt(scope.clone(), "vendorx", &thread_scope);
    request.intent = DeliveryIntent::Working;
    let error = coordinator
        .deliver(&policy, &resolver, request)
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
            prune_registrations: Vec::new(),
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
async fn coordinator_does_not_auto_recover_unfenced_sending_attempts() {
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
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
            projection_ref: ironclaw_outbound::ProjectionUpdateRef::new("projection:live-send")
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
        .expect("seed sending attempt");
    let adapter = Arc::new(ScriptedChannelAdapter::new(
        Arc::clone(&store),
        scope.clone(),
        vec![Ok(DeliveryReport {
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-new")],
        })],
    ));

    coordinator_over(&store, &adapter)
        .deliver_notice(working_notice(scope.clone(), "vendorx"))
        .await
        .expect("new notice delivers");

    let persisted = store
        .load_delivery_attempt(scope, stray.delivery_id)
        .await
        .expect("load succeeds")
        .expect("sending attempt remains");
    assert_eq!(
        persisted.status,
        ironclaw_outbound::OutboundDeliveryStatus::Sending,
        "without a durable owner lease, another coordinator must not guess that a send crashed"
    );
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
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Message),
            requires_enrollment: false,
        }),
        Arc::new(FixedReplyContext::new(Vec::new())),
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
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
                prune_registrations: Vec::new(),
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
            thread_anchor: None,
            require_direct_message_target: true,
            extension_id: "vendorx",
            thread_scope: &thread_scope,
        };

        let outcome = coordinator.deliver(&policy, &resolver, request).await;

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

// ─── The reply sink is never a coordinator half ─────────────────────────────
//
// A channel's `[channel.reply]` binds one reply sink, reconciled by reply
// publication against the run's reply document. The coordinator drives
// discrete sends only — prompts and notices on the source conversation,
// preference-target notifications, model deliveries — and every one of them
// goes out through the delivery half. A channel that bound only its reply
// sink therefore cannot carry a coordinator send: the attempt fails closed
// instead of being pushed through the sink as if it were a message half.

struct ReplySinkOnlyResolver {
    sink: Arc<RecordingReplySink>,
}

impl ChannelDeliveryResolver for ReplySinkOnlyResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        Some(ResolvedChannelDelivery {
            extension_id: ExtensionId::new(extension_id).expect("valid extension id"),
            installation_id: AdapterInstallationId::new("inst-1").expect("valid installation id"),
            reply: Some(Arc::clone(&self.sink) as Arc<dyn ReplySink>),
            delivery: None,
            egress: Arc::new(CoordinatorDenyAllEgress),
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Stream),
            requires_enrollment: false,
            declared_egress_hosts: Vec::new(),
            generation: 0,
        })
    }
}

#[tokio::test]
async fn a_reply_route_send_never_reaches_the_reply_sink_and_fails_closed_without_a_delivery_half()
{
    let scope = scope();
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
    let preferences = FakePreferenceRepository::default();
    seed_preference(&preferences, &scope);
    let resolver = FakeProductOutboundTargetResolver;
    let policy = configured_policy(&store, &validator);
    let sink = Arc::new(RecordingReplySink::default());
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(ReplySinkOnlyResolver {
            sink: Arc::clone(&sink),
        }),
        Arc::new(FixedReplyContext::new(b"vendor-reply-ctx".to_vec()))
            as Arc<dyn DeliveryReplyContextSource>,
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    );

    let thread_scope = project_thread_scope();
    let request = coordinated_gate_prompt(scope.clone(), "vendorx", &thread_scope);
    let outcome = coordinator
        .deliver(&policy, &resolver, request)
        .await
        .expect("a missing delivery half is a settled outcome, not a transport error");

    assert!(
        matches!(
            outcome,
            CoordinatedDeliveryOutcome::Failed {
                failure_kind: ironclaw_outbound::DeliveryFailureKind::Rejected,
                ..
            }
        ),
        "a coordinator send has no half to ride on a reply-sink-only channel, got {outcome:?}"
    );
    assert!(
        sink.requests().is_empty(),
        "the reply sink is reply publication's, never a coordinator send half"
    );
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts.len(),
        1,
        "the attempt row is still the audit record"
    );
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Failed,
        "fail closed: nothing was sent and nothing pretends it was"
    );
}

#[tokio::test]
async fn streaming_channel_still_receives_notification_class_deliveries() {
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-200")],
        })],
    ));
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: false,
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Stream),
            requires_enrollment: false,
        }),
        Arc::new(FixedReplyContext::new(b"vendor-reply-ctx".to_vec()))
            as Arc<dyn DeliveryReplyContextSource>,
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    );

    let thread_scope = project_thread_scope();
    let mut request = coordinated_gate_prompt(scope.clone(), "vendorx", &thread_scope);
    request.intent = DeliveryIntent::ModelDelivery;
    request.delivery.resolution_request.intent =
        ironclaw_outbound::CommunicationDeliveryIntent::RequestedOutbound(
            ironclaw_outbound::RequestedOutboundContext {
                requested_target: validated_reply_target(),
                requested_kind: ironclaw_outbound::RequestedOutboundKind::ProductMessage,
            },
        );
    let outcome = coordinator
        .deliver(&policy, &resolver, request)
        .await
        .expect("notification-class delivery drives");

    assert!(
        matches!(outcome, CoordinatedDeliveryOutcome::Delivered { .. }),
        "notification-class sends must flow to a streaming channel, got {outcome:?}"
    );
    assert_eq!(adapter.deliver_calls(), 1);
}

/// Regression pin (unified-channel-model §5/§7a): a gate prompt whose ROUTE
/// is a notification target (`RunNotification` + non-live-source origin) is a
/// notification even though its `DeliveryIntent` is conversation-shaped. The
/// streaming gate keys on the route, not the intent — skipping this send
/// would silently drop blocked-fire pushes for a streaming channel, the exact
/// break `blocked_fire_pushes_web_app_notice_to_enrolled_browser` caught.
#[tokio::test]
async fn streaming_channel_delivers_a_notification_routed_gate_prompt() {
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-300")],
        })],
    ));
    let coordinator = DeliveryCoordinator::new(
        Arc::clone(&store) as Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: false,
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Stream),
            requires_enrollment: false,
        }),
        Arc::new(FixedReplyContext::new(b"vendor-reply-ctx".to_vec()))
            as Arc<dyn DeliveryReplyContextSource>,
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    );

    let thread_scope = project_thread_scope();
    let mut request = coordinated_notification(scope.clone(), "vendorx", &thread_scope);
    // Conversation-shaped intent, notification-shaped route: the gate prompt
    // for a background fire, pushed to the creator's notification channel.
    request.intent = DeliveryIntent::GatePrompt;
    let outcome = coordinator
        .deliver(&policy, &resolver, request)
        .await
        .expect("notification-routed gate prompt drives");

    assert!(
        matches!(outcome, CoordinatedDeliveryOutcome::Delivered { .. }),
        "a notification-routed gate prompt must flow to a streaming channel, got {outcome:?}"
    );
    assert_eq!(
        adapter.delivery_sends(),
        1,
        "the notification route must ride the adapter's notification send"
    );
}

// ─── §7a adapter dispatch: notifications ride ChannelDelivery ──────────────

fn coordinated_notification<'a>(
    scope: TurnScope,
    extension_id: &'a str,
    thread_scope: &'a ThreadScope,
) -> CoordinatedDeliveryRequest<'a> {
    let mut request = coordinated_gate_prompt(scope.clone(), extension_id, thread_scope);
    request.intent = DeliveryIntent::BackgroundRunNotice;
    request.delivery.resolution_request.intent =
        ironclaw_outbound::CommunicationDeliveryIntent::RunNotification(
            ironclaw_outbound::RunNotificationContext {
                event_kind: ironclaw_outbound::RunNotificationEventKind::RunBlocked,
                origin: ironclaw_outbound::RunNotificationOrigin::RunScopedTarget {
                    target: validated_reply_target(),
                },
            },
        );
    request
}

#[tokio::test]
async fn notification_class_delivery_rides_the_adapters_notification_send() {
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-300")],
        })],
    ));
    let (coordinator, _reply_context) = coordinator_over_recording_reply_lookups(&store, &adapter);

    let thread_scope = project_thread_scope();
    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_notification(scope.clone(), "vendorx", &thread_scope),
        )
        .await
        .expect("notification delivers");

    assert!(matches!(
        outcome,
        CoordinatedDeliveryOutcome::Delivered { .. }
    ));
    assert_eq!(
        adapter.delivery_sends(),
        1,
        "a run notification must ride ChannelDelivery::deliver"
    );
}

#[tokio::test]
async fn enrollment_required_without_registrations_records_no_target_without_egress() {
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
            unavailable: false,
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Message),
            requires_enrollment: true,
        }),
        Arc::new(FixedReplyContext::new(Vec::new())),
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    );

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_notification(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect("no enrolled target is a clean outcome");

    assert!(matches!(outcome, CoordinatedDeliveryOutcome::NoDelivery));
    assert_eq!(adapter.delivery_sends(), 0, "no adapter call is evidence");
    let attempts = store
        .list_delivery_attempts(scope)
        .await
        .expect("list attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::NoTarget,
        "absence of a target is not provider-confirmed delivery"
    );
}

#[tokio::test]
async fn enrollment_required_ownerless_delivery_records_no_target_without_egress() {
    let scope = TurnScope::new_with_owner(
        TenantId::new("tenant-product-outbound").expect("valid tenant"),
        Some(AgentId::new("agent-product-outbound").expect("valid agent")),
        Some(ProjectId::new("project-product-outbound").expect("valid project")),
        ThreadId::new("thread-ownerless-outbound").expect("valid thread"),
        None,
    );
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let validator = FakeReplyTargetBindingValidator::default();
    validator.allow(validated_reply_target());
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
            unavailable: false,
            reply_transport: Some(ironclaw_extension_contracts::channel::ReplyTransport::Message),
            requires_enrollment: true,
        }),
        Arc::new(FixedReplyContext::new(Vec::new())),
        Arc::new(ironclaw_assistant::NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 3,
            backoff: std::time::Duration::ZERO,
        },
    );

    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_notification(scope.clone(), "vendorx", &project_thread_scope()),
        )
        .await
        .expect("ownerless enrollment-required delivery has no target");

    assert!(matches!(outcome, CoordinatedDeliveryOutcome::NoDelivery));
    assert_eq!(adapter.delivery_sends(), 0, "no adapter call is evidence");
    let attempts = store
        .list_delivery_attempts(scope)
        .await
        .expect("list attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::NoTarget,
    );
}

#[tokio::test]
async fn a_source_routed_prompt_rides_the_channels_delivery_half() {
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
            prune_registrations: Vec::new(),
            parts: vec![sent("ts-301")],
        })],
    ));
    let (coordinator, _reply_context) = coordinator_over_recording_reply_lookups(&store, &adapter);

    let thread_scope = project_thread_scope();
    let outcome = coordinator
        .deliver(
            &policy,
            &resolver,
            coordinated_gate_prompt(scope.clone(), "vendorx", &thread_scope),
        )
        .await
        .expect("prompt delivers");

    assert!(matches!(
        outcome,
        CoordinatedDeliveryOutcome::Delivered { .. }
    ));
    assert_eq!(
        adapter.delivery_sends(),
        1,
        "a source-routed prompt is a discrete message on the channel's delivery half; the reply sink belongs to reply publication"
    );
}

// ── §8: the generic notification-setup surface over host-owned registrations ─
//
// The block that stood here drove `AdapterChannelNotificationSetupService`
// against a scripted adapter, pinning that the service passed the caller's
// identity and the channel-opaque payload through verbatim. Those adapter
// methods are gone (design §8), and the two properties split:
//
//   * The caller's identity is still never reinterpreted — the scope is built
//     from `ProductSurfaceCaller` alone, pinned below.
//   * The opaque payload is now stored host-side and parsed only at delivery,
//     pinned in `ironclaw_auth::delivery_registrations` (storage bounds, the
//     endpoint allowlist, the forward migration) and in the web-app package's
//     `registration_parsing_contract` (interpretation).
//
// What is genuinely THIS layer's, and therefore what is tested here, is the
// pre-storage endpoint admission: the surface must refuse an endpoint the
// channel's own `[[channel.egress]]` does not declare, before anything is
// written. Without that check enrollment is an SSRF primitive.

struct EnrollmentResolver {
    requires_enrollment: Option<bool>,
    declared_hosts: Option<Vec<String>>,
}

impl ChannelDeliveryResolver for EnrollmentResolver {
    fn resolve_channel_delivery(
        &self,
        extension_id: &str,
    ) -> Option<ironclaw_product_contracts::delivery::ResolvedChannelDelivery> {
        Some(
            ironclaw_product_contracts::delivery::ResolvedChannelDelivery {
                extension_id: ExtensionId::new(extension_id).ok()?,
                installation_id: AdapterInstallationId::new("enrollment-test").ok()?,
                reply: None,
                delivery: None,
                egress: Arc::new(CoordinatorDenyAllEgress),
                reply_transport: None,
                requires_enrollment: self.requires_enrollment?,
                declared_egress_hosts: self.declared_hosts.clone()?,
                generation: 0,
            },
        )
    }
}

#[derive(Default)]
struct RecordingRegistrations {
    enrolled: Mutex<Vec<String>>,
    listed: Mutex<Vec<ironclaw_extension_contracts::channel_adapter::DeliveryRegistration>>,
    removed: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ironclaw_product_contracts::delivery::DeliveryRegistrationService for RecordingRegistrations {
    async fn list(
        &self,
        _scope: &ironclaw_product_contracts::delivery::DeliveryRegistrationScope,
    ) -> Result<
        Vec<ironclaw_extension_contracts::channel_adapter::DeliveryRegistration>,
        ironclaw_product_contracts::delivery::DeliveryRegistrationError,
    > {
        Ok(self.listed.lock().expect("listed lock").clone())
    }

    async fn enroll(
        &self,
        _scope: &ironclaw_product_contracts::delivery::DeliveryRegistrationScope,
        request: ironclaw_product_contracts::delivery::DeliveryRegistrationRequest,
    ) -> Result<
        ironclaw_extension_contracts::channel_adapter::DeliveryRegistration,
        ironclaw_product_contracts::delivery::DeliveryRegistrationError,
    > {
        self.enrolled
            .lock()
            .expect("enrolled lock")
            .push(request.endpoint.clone());
        let registration = ironclaw_extension_contracts::channel_adapter::DeliveryRegistration {
            registration_id: "reg-1".to_string(),
            endpoint: request.endpoint,
            document: request.document,
            created_at: "2026-08-11T00:00:00Z".to_string(),
        };
        self.listed
            .lock()
            .expect("listed lock")
            .push(registration.clone());
        Ok(registration)
    }

    async fn remove(
        &self,
        _scope: &ironclaw_product_contracts::delivery::DeliveryRegistrationScope,
        registration_id: &str,
    ) -> Result<bool, ironclaw_product_contracts::delivery::DeliveryRegistrationError> {
        self.removed
            .lock()
            .expect("removed lock")
            .push(registration_id.to_string());
        self.listed
            .lock()
            .expect("listed lock")
            .retain(|registration| registration.registration_id != registration_id);
        Ok(true)
    }

    async fn prune(
        &self,
        _scope: &ironclaw_product_contracts::delivery::DeliveryRegistrationScope,
        _registration_ids: &[String],
    ) -> Result<usize, ironclaw_product_contracts::delivery::DeliveryRegistrationError> {
        Ok(0)
    }
}

fn setup_caller() -> ironclaw_product_contracts::surface::ProductSurfaceCaller {
    ironclaw_product_contracts::surface::ProductSurfaceCaller::new(
        ironclaw_host_api::ids::TenantId::new("tenant1").expect("tenant"),
        ironclaw_host_api::ids::UserId::new("user1").expect("user"),
        None,
        None,
    )
}

fn enrollment_service(
    requires_enrollment: Option<bool>,
    declared_hosts: Option<Vec<String>>,
    registrations: Arc<RecordingRegistrations>,
) -> ironclaw_assistant::RegistrationChannelNotificationSetupService {
    ironclaw_assistant::RegistrationChannelNotificationSetupService::new(
        Arc::new(EnrollmentResolver {
            requires_enrollment,
            declared_hosts,
        }),
        registrations,
        Arc::new(ironclaw_assistant::NoDeliveryClientBootstrap),
    )
}

struct FailingDeliveryClientBootstrap;

impl ironclaw_assistant::DeliveryClientBootstrap for FailingDeliveryClientBootstrap {
    fn bootstrap(
        &self,
        _extension_id: &str,
    ) -> Result<Option<serde_json::Value>, ironclaw_assistant::DeliveryClientBootstrapError> {
        Err(ironclaw_assistant::DeliveryClientBootstrapError)
    }
}

fn enrollment_payload(endpoint: &str) -> serde_json::Value {
    serde_json::json!({ "endpoint": endpoint, "keys": { "p256dh": "a", "auth": "b" } })
}

#[tokio::test]
async fn notification_status_surfaces_bootstrap_failure_as_retryable_unavailability() {
    use ironclaw_assistant::ChannelNotificationSetupService as _;

    let service = ironclaw_assistant::RegistrationChannelNotificationSetupService::new(
        Arc::new(EnrollmentResolver {
            requires_enrollment: Some(true),
            declared_hosts: Some(vec!["push.declared.example".to_string()]),
        }),
        Arc::new(RecordingRegistrations::default()),
        Arc::new(FailingDeliveryClientBootstrap),
    );

    let error = service
        .status(
            setup_caller(),
            ironclaw_product_contracts::product_wire::RebornNotificationSetupRequest {
                extension_id: "vendorx".to_string(),
            },
        )
        .await
        .expect_err("bootstrap failure must not be hidden as missing bootstrap data");

    assert_eq!(
        error.kind,
        ironclaw_product_contracts::surface::ProductSurfaceErrorKind::ServiceUnavailable
    );
    assert!(error.retryable);
}

/// THE security-critical check, at the surface that performs it: an endpoint
/// the channel does not declare must be refused BEFORE storage. Every hostile
/// shape here would otherwise make the host POST wherever the submitter named.
#[tokio::test]
async fn enrollment_refuses_an_undeclared_endpoint_before_storage() {
    use ironclaw_assistant::ChannelNotificationSetupService as _;

    let registrations = Arc::new(RecordingRegistrations::default());
    let service = enrollment_service(
        Some(true),
        Some(vec!["push.declared.example".to_string()]),
        Arc::clone(&registrations),
    );

    for hostile in [
        "https://evil.example/send/x",
        "http://push.declared.example/send/x",
        "https://push.declared.example@evil.example/send/x",
        "https://push.declared.example.evil.example/send/x",
    ] {
        let error = service
            .enable(
                setup_caller(),
                ironclaw_product_contracts::product_wire::RebornNotificationSetupMutationRequest {
                    extension_id: "vendorx".to_string(),
                    payload: enrollment_payload(hostile),
                },
            )
            .await
            .expect_err("an undeclared endpoint must be refused");
        assert_eq!(
            error.kind,
            ironclaw_product_contracts::surface::ProductSurfaceErrorKind::Validation,
            "{hostile}"
        );
    }

    assert!(
        registrations.enrolled.lock().expect("lock").is_empty(),
        "nothing may be written for a refused endpoint"
    );
}

#[tokio::test]
async fn enrollment_stores_a_declared_endpoint_with_its_opaque_document() {
    use ironclaw_assistant::ChannelNotificationSetupService as _;

    let registrations = Arc::new(RecordingRegistrations::default());
    let service = enrollment_service(
        Some(true),
        Some(vec!["push.declared.example".to_string()]),
        Arc::clone(&registrations),
    );
    service
        .enable(
            setup_caller(),
            ironclaw_product_contracts::product_wire::RebornNotificationSetupMutationRequest {
                extension_id: "vendorx".to_string(),
                payload: enrollment_payload("https://push.declared.example/send/ok"),
            },
        )
        .await
        .expect("a declared endpoint enrolls");
    assert_eq!(
        *registrations.enrolled.lock().expect("lock"),
        vec!["https://push.declared.example/send/ok".to_string()]
    );
}

#[tokio::test]
async fn disable_resolves_the_edge_endpoint_to_the_canonical_host_registration_id() {
    use ironclaw_assistant::ChannelNotificationSetupService as _;

    let registrations = Arc::new(RecordingRegistrations::default());
    registrations.listed.lock().expect("listed lock").push(
        ironclaw_extension_contracts::channel_adapter::DeliveryRegistration {
            registration_id: "host-registration-7".to_string(),
            endpoint: "https://push.declared.example/send/old".to_string(),
            document: r#"{"keys":{"p256dh":"a","auth":"b"}}"#.to_string(),
            created_at: "2026-08-11T00:00:00Z".to_string(),
        },
    );
    let service = enrollment_service(
        Some(true),
        Some(vec!["push.declared.example".to_string()]),
        Arc::clone(&registrations),
    );

    service
        .disable(
            setup_caller(),
            ironclaw_product_contracts::product_wire::RebornNotificationSetupMutationRequest {
                extension_id: "vendorx".to_string(),
                payload: enrollment_payload("https://push.declared.example/send/old"),
            },
        )
        .await
        .expect("the edge endpoint resolves to its stored canonical id");

    assert_eq!(
        *registrations.removed.lock().expect("removed lock"),
        vec!["host-registration-7".to_string()]
    );
}

/// A channel the deployment does not know must not become an oracle for which
/// channels exist, and one with no declared egress can enroll nothing.
#[tokio::test]
async fn unknown_and_egressless_channels_fail_closed() {
    use ironclaw_assistant::ChannelNotificationSetupService as _;

    for (requires_enrollment, declared_hosts) in [
        (None, Some(vec!["push.declared.example".to_string()])),
        (Some(true), None),
    ] {
        let service = enrollment_service(
            requires_enrollment,
            declared_hosts,
            Arc::new(RecordingRegistrations::default()),
        );
        let error = service
            .enable(
                setup_caller(),
                ironclaw_product_contracts::product_wire::RebornNotificationSetupMutationRequest {
                    extension_id: "vendorx".to_string(),
                    payload: enrollment_payload("https://push.declared.example/send/x"),
                },
            )
            .await
            .expect_err("neither an unknown nor an egressless channel may enroll");
        assert_eq!(
            error.kind,
            ironclaw_product_contracts::surface::ProductSurfaceErrorKind::NotFound,
            "both paths must be indistinguishable: {requires_enrollment:?}"
        );
    }
}

/// Delegating store whose terminal `Delivered` status write fails — the
/// durable shape behind theredspoon's #7157 flag (and #7029's fix on main):
/// vendor egress succeeded, but the confirmation row never committed.
struct TerminalDeliveredWriteFailingStore {
    inner: Arc<OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    /// Simulates a send worker committing `Delivered` after recovery's list
    /// snapshot but before its guarded `Sending -> Unknown` transition.
    complete_before_recovery: bool,
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
        if self.complete_before_recovery {
            self.inner
                .update_delivery_status(ironclaw_outbound::UpdateDeliveryStatusRequest {
                    delivery_id: request.delivery_id,
                    scope: request.scope.clone(),
                    status: ironclaw_outbound::OutboundDeliveryStatus::Delivered,
                    updated_at: Utc::now(),
                    failure_kind: None,
                })
                .await?;
        }
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
    async fn open_reply_publication(
        &self,
        request: ironclaw_outbound::OpenReplyPublicationRequest,
    ) -> Result<ironclaw_outbound::ReplyPublicationRecord, OutboundError> {
        self.inner.open_reply_publication(request).await
    }

    async fn claim_reply_publication_lease(
        &self,
        request: ironclaw_outbound::ClaimReplyPublicationLeaseRequest,
    ) -> Result<ironclaw_outbound::ReplyPublicationClaim, OutboundError> {
        self.inner.claim_reply_publication_lease(request).await
    }

    async fn advance_reply_publication(
        &self,
        request: ironclaw_outbound::AdvanceReplyPublicationRequest,
    ) -> Result<ironclaw_outbound::ReplyPublicationRecord, OutboundError> {
        self.inner.advance_reply_publication(request).await
    }

    async fn settle_reply_publication(
        &self,
        request: ironclaw_outbound::SettleReplyPublicationRequest,
    ) -> Result<ironclaw_outbound::ReplyPublicationRecord, OutboundError> {
        self.inner.settle_reply_publication(request).await
    }

    async fn release_reply_publication_lease(
        &self,
        request: ironclaw_outbound::ReleaseReplyPublicationLeaseRequest,
    ) -> Result<(), OutboundError> {
        self.inner.release_reply_publication_lease(request).await
    }

    async fn load_reply_publication(
        &self,
        scope: TurnScope,
        delivery_id: ironclaw_outbound::OutboundDeliveryId,
    ) -> Result<Option<ironclaw_outbound::ReplyPublicationRecord>, OutboundError> {
        self.inner.load_reply_publication(scope, delivery_id).await
    }

    async fn list_reply_publications(
        &self,
        scope: TurnScope,
        run_id: ironclaw_host_api::turn::TurnRunId,
    ) -> Result<Vec<ironclaw_outbound::ReplyPublicationRecord>, OutboundError> {
        self.inner.list_reply_publications(scope, run_id).await
    }

    async fn list_open_reply_publications(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<ironclaw_outbound::ReplyPublicationRecord>, OutboundError> {
        self.inner.list_open_reply_publications(scope).await
    }
}
