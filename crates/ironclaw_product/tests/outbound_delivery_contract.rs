// arch-exempt: large_file, mechanical FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend> -> FilesystemOutboundStateStore<InMemoryBackend> §4.3 store consolidation, no logic change, plan #6168
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    CommunicationPreferenceKey, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    CommunicationPreferenceVersion, DeliveryDefaultScope, FilesystemOutboundStateStore,
    OutboundDeliveryAttempt, OutboundError, OutboundPolicyService, OutboundStateStore,
    ReplyTargetBindingClaim, ReplyTargetBindingValidator, RunNotificationContext,
    RunNotificationEventKind, RunNotificationOrigin, ThreadProjectionAccessClaim,
    ThreadProjectionAccessPolicy, ThreadProjectionAccessRequest, TriggerFireSlot, TriggerOriginRef,
    TriggerSourceKind, VersionedCommunicationPreferenceRecord, WriteCommunicationPreferenceRequest,
};
use ironclaw_product::{ExternalActorRef, ExternalConversationRef};
use ironclaw_product::{
    ProductOutboundTargetResolver, ProductWorkflowError, VerifiedProductOutboundTargetMetadata,
};
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
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
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

fn trigger_context() -> ironclaw_outbound::TriggerCommunicationContext {
    ironclaw_outbound::TriggerCommunicationContext {
        trigger_origin_ref: TriggerOriginRef::new("trigger-origin:product-outbound")
            .expect("valid trigger origin ref"),
        trigger_source_kind: TriggerSourceKind::Schedule,
        fire_slot: TriggerFireSlot::new("fire-slot:product-outbound")
            .expect("valid trigger fire slot"),
    }
}

fn configured_policy<'a>(
    store: &'a FilesystemOutboundStateStore<InMemoryBackend>,
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

// ---------------------------------------------------------------------------
// Delivery coordinator (extension-runtime §5.4; OUT-1..7, ING-11)
// ---------------------------------------------------------------------------

use std::collections::VecDeque;
use std::sync::Arc;

use ironclaw_product::{
    ChannelAdapter, ChannelError, DeliveryReport, InboundOutcome, OutboundEnvelope,
    PartDeliveryOutcome, VerifiedInbound,
};
use ironclaw_product::{
    ChannelDeliveryResolver, CoordinatedDeliveryError, CoordinatedDeliveryOutcome,
    CoordinatedDeliveryRequest, DeliveryCoordinator, DeliveryIntent, DeliveryReplyContextSource,
    DeliveryRetryPolicy, NoticeDeliveryRequest, ResolvedChannelDelivery,
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
    store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    scope: TurnScope,
}

impl ScriptedChannelAdapter {
    fn new(
        store: Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
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
    store: &Arc<FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
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
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
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

fn coordinated_final_reply(scope: TurnScope, extension_id: &str) -> CoordinatedDeliveryRequest<'_> {
    CoordinatedDeliveryRequest {
        intent: DeliveryIntent::FinalReply,
        delivery: delivery_request(scope),
        parts: vec![ironclaw_product::OutboundPart::Text(
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
    let attempts = store.list_delivery_attempts(scope).await.unwrap();
    assert_eq!(
        attempts[0].status,
        ironclaw_outbound::OutboundDeliveryStatus::Delivered
    );
}

#[tokio::test]
async fn coordinator_suppresses_the_same_projection_after_reopen() {
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
            parts: vec![sent("ts-reopen")],
        })],
    ));
    let delivery = delivery_request(scope.clone());

    let first = coordinator_over(&store, &adapter);
    let first_outcome = first
        .deliver(
            &policy,
            &preferences,
            &resolver,
            CoordinatedDeliveryRequest {
                intent: DeliveryIntent::FinalReply,
                delivery: delivery.clone(),
                parts: vec![ironclaw_product::OutboundPart::Text(
                    "one durable fact".to_string(),
                )],
                thread_anchor: None,
                require_direct_message_target: false,
                extension_id: "vendorx",
            },
        )
        .await
        .expect("first delivery succeeds");
    assert!(matches!(
        first_outcome,
        CoordinatedDeliveryOutcome::Delivered { .. }
    ));

    // A new coordinator models a reopened process over the same durable
    // store. The stable projection identity resolves to the same attempt, and
    // the atomic Prepared -> Sending claim cannot be acquired twice.
    let reopened = coordinator_over(&store, &adapter);
    let replay_outcome = reopened
        .deliver(
            &policy,
            &preferences,
            &resolver,
            CoordinatedDeliveryRequest {
                intent: DeliveryIntent::FinalReply,
                delivery,
                parts: vec![ironclaw_product::OutboundPart::Text(
                    "one durable fact".to_string(),
                )],
                thread_anchor: None,
                require_direct_message_target: false,
                extension_id: "vendorx",
            },
        )
        .await
        .expect("replay is safely suppressed");

    assert!(matches!(
        replay_outcome,
        CoordinatedDeliveryOutcome::DuplicateSuppressed { .. }
    ));
    assert_eq!(adapter.deliver_calls(), 1);
    assert_eq!(store.list_delivery_attempts(scope).await.unwrap().len(), 1);
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

    let request = CoordinatedDeliveryRequest {
        intent: DeliveryIntent::FinalReply,
        delivery: delivery_request(scope.clone()),
        parts: vec![ironclaw_product::OutboundPart::Text("dm only".to_string())],
        thread_anchor: Some("thread-1".to_string()),
        require_direct_message_target: true,
        extension_id: "vendorx",
    };
    let error = coordinator
        .deliver(&policy, &preferences, &resolver, request)
        .await
        .expect_err("require_direct_message=true against a non-DM target must reject");
    assert!(
        matches!(
            error,
            CoordinatedDeliveryError::Workflow(
                ProductWorkflowError::OutboundTargetNotDirectMessage
            )
        ),
        "unexpected error: {error:?}"
    );
    // Fail-closed BEFORE any vendor egress: the channel adapter never delivered.
    assert_eq!(adapter.deliver_calls(), 0);
    // Audit records Rejected (not Unknown) — the #4953 failure-kind mapping,
    // via `delivery_failure_kind_for_workflow_error`.
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
            &preferences,
            &resolver,
            coordinated_final_reply(scope.clone(), "vendorx"),
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
        parts: vec![ironclaw_product::OutboundPart::Text(
            "Working on it...".to_string(),
        )],
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

    let mut request = working_notice(scope.clone(), "vendorx");
    request.intent = DeliveryIntent::FinalReply;
    let error = coordinator
        .deliver_notice(request)
        .await
        .expect_err("policy-class intents must use the policy path");
    assert!(matches!(
        error,
        CoordinatedDeliveryError::IntentClassMismatch { .. }
    ));
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

    let mut request = coordinated_final_reply(scope.clone(), "vendorx");
    request.intent = DeliveryIntent::Working;
    let error = coordinator
        .deliver(&policy, &preferences, &resolver, request)
        .await
        .expect_err("notice-class intents must use the notice path");
    assert!(matches!(
        error,
        CoordinatedDeliveryError::IntentClassMismatch { .. }
    ));
    assert_eq!(adapter.deliver_calls(), 0);
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
    request.parts = vec![ironclaw_product::OutboundPart::Retract {
        vendor_message_ref: "ts-900".to_string(),
    }];
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
        [ironclaw_product::OutboundPart::Retract { vendor_message_ref }]
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
        Arc::clone(&store) as Arc<dyn ironclaw_outbound::OutboundStateStore>,
        Arc::new(StaticChannelResolver {
            adapter: Arc::clone(&adapter),
            unavailable: true,
        }),
        Arc::new(FixedReplyContext(Vec::new())),
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
