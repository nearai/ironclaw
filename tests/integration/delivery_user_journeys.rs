//! Caller-level delivery routing matrix.
//!
//! The existing extension-delivery suite owns real implicit-source Slack and
//! Telegram ingress -> run -> bot-reply wire proofs:
//! `slack_final_reply_flows_through_the_real_delivery_coordinator` and
//! `telegram_update_becomes_a_turn_and_a_coordinated_reply`. This target owns
//! the complementary user journeys:
//!
//! - immediate one-off Telegram -> Slack and Slack -> Telegram results persist
//!   an exact opaque target, then cross the lifecycle handler, coordinator, and
//!   destination bot wire without changing the user-wide default;
//! - the host-owned `web_app` target keeps the result in WebUI and performs no
//!   external delivery;
//! - explicit scheduled targets use the same coordinator path; omitted-target
//!   source inheritance is proved separately through real `trigger_create` in
//!   `group_triggers::scenario_external_source_trigger_captures_delivery`;
//! - removed and foreign targets fail closed; and
//! - a selected scheduled target survives an auth block and resume.
//!
//! The model-operation tests require a successful
//! `builtin.outbound_delivery_target_route_current` result, while the real
//! cross-channel tests separately read back the host-sealed run target before
//! the final lifecycle event. This keeps failures pinned to the precise seam:
//! mediated first-party handler/product-store wiring versus consumption by
//! final-reply delivery.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
};
use ironclaw_host_api::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_outbound::{
    CommunicationModality, CommunicationPreferenceRecord, DeliveryDefaultScope,
    DeliveryFailureKind, OutboundDeliveryStatus, OutboundStateStore, RunFinalReplyDestination,
    RunFinalReplyTargetRecord, RunFinalReplyTargetRequest, TriggerCommunicationContext,
    TriggerFireSlot, TriggerOriginRef, TriggerSourceKind, TriggeredRunDeliveryOutcomeKind,
    TriggeredRunDeliveryStore, WriteCommunicationPreferenceRequest,
};
use ironclaw_product::{
    AdapterInstallationId, AuthPromptChallengeKind, AuthPromptView, AuthRequirement,
    ChannelAdapter, ExternalConversationRef, InboundOutcome, ParsedProductInbound,
    ProductAdapterError, ProductAdapterId, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProtocolAuthEvidence, TrustedInboundContext, UserMessagePayload,
    VerifiedInbound,
};
use ironclaw_product::{
    BlockedAuthPromptRequest, BlockedAuthPromptSource, ChannelDeliveryResolver,
    ConversationBindingService, CurrentDeliveryTarget, CurrentDeliveryTargetResolver,
    DeliveryCoordinator, DeliveryReplyContextSource, DeliveryRetryPolicy,
    OUTBOUND_DELIVERY_TARGETS_VIEW, OutboundPreferencesProductFacade, ProductConversationRouteKind,
    ProductCreateThreadRequest, ProductSubmitTurnRequest, ProductWorkflowError,
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetListResponse, RebornOutboundDeliveryTargetOption,
    RebornOutboundDeliveryTargetSummary, RebornOutboundPreferencesResponse, RebornServices,
    RebornSetOutboundPreferencesRequest, RebornSubmitTurnResponse, RebornViewQuery,
    ResolveBindingRequest, ResolveStoredProductReplyTargetRequest, ResolvedBinding,
    ResolvedChannelDelivery, ResolvedStoredProductReplyTarget, RunDeliveryEventHandler,
    RunDeliveryEventRouter, RunDeliveryServices, TriggeredRunDeliveryDriver,
    TriggeredRunDeliveryRequest,
};
use ironclaw_turns::{
    GateResumeDisposition, GetRunStateRequest, ReplyTargetBindingRef, ResumeTurnPrecondition,
    TurnActor, TurnEventKind, TurnEventSink, TurnLifecycleEvent, TurnOriginKind, TurnStateStore,
    TurnStatus,
};
use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::ParkingModelGate;
use serde_json::{Value, json};

const TARGETS_LIST: &str = "builtin.outbound_delivery_targets_list";
const ROUTE_CURRENT: &str = "builtin.outbound_delivery_target_route_current";
const TRIGGER_CREATE: &str = "builtin.trigger_create";

#[derive(Clone, Copy)]
enum ChannelKind {
    Slack,
    Telegram,
}

impl ChannelKind {
    fn extension_id(self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Telegram => "telegram",
        }
    }

    fn adapter(self) -> Arc<dyn ChannelAdapter> {
        match self {
            Self::Slack => Arc::new(ironclaw_slack_extension::SlackChannelAdapter),
            Self::Telegram => {
                Arc::new(ironclaw_telegram_extension::TelegramChannelAdapter::default())
            }
        }
    }

    fn target(self) -> (ReplyTargetBindingRef, ExternalConversationRef) {
        match self {
            Self::Slack => (
                ReplyTargetBindingRef::new("reply:delivery-matrix-slack-dm")
                    .expect("Slack target ref"),
                ExternalConversationRef::new(Some("T-MATRIX"), "D-MATRIX", None, None)
                    .expect("Slack conversation"),
            ),
            Self::Telegram => (
                ReplyTargetBindingRef::new("reply:delivery-matrix-telegram-dm")
                    .expect("Telegram target ref"),
                ExternalConversationRef::new(None::<&str>, "8675309", None, None)
                    .expect("Telegram conversation"),
            ),
        }
    }

    fn inbound_body(self, label: &str) -> String {
        match self {
            Self::Slack => json!({
                "type": "event_callback",
                "event_id": format!("Ev-delivery-matrix-{label}"),
                "team_id": "T-MATRIX",
                "event": {
                    "type": "message",
                    "user": "U-MATRIX",
                    "channel": "D-MATRIX-SOURCE",
                    "channel_type": "im",
                    "text": "route this result",
                    "ts": "1710000300.000100"
                }
            })
            .to_string(),
            Self::Telegram => json!({
                "update_id": 99001,
                "message": {
                    "message_id": 901,
                    "date": 1710000000,
                    "text": "route this result",
                    "from": {"id": 9911, "is_bot": false, "first_name": "Ada"},
                    "chat": {"id": 8675310, "type": "private"}
                }
            })
            .to_string(),
        }
    }

    fn inbound_envelope(self, label: &str) -> ProductInboundEnvelope {
        let body = self.inbound_body(label);
        let outcome = self
            .adapter()
            .inbound(VerifiedInbound {
                extension_id: self.extension_id(),
                installation_id: "delivery-matrix-installation",
                body: body.as_bytes(),
                headers: &[],
            })
            .expect("real channel adapter accepts the source payload");
        let InboundOutcome::Messages(messages) = outcome else {
            panic!("real source payload must normalize to a user message");
        };
        let message = messages
            .into_iter()
            .next()
            .expect("one real source message is normalized");
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::BearerToken,
            "delivery-matrix-verified-caller",
        );
        let context = TrustedInboundContext::from_verified_evidence(
            ProductAdapterId::new(self.extension_id()).expect("source adapter id"),
            AdapterInstallationId::new("delivery-matrix-installation")
                .expect("source installation id"),
            Utc::now(),
            &evidence,
        )
        .expect("verified source context");
        let payload = ProductInboundPayload::UserMessage(
            UserMessagePayload::new(message.text, Vec::new(), message.trigger)
                .expect("normalized source message payload"),
        );
        let parsed = ParsedProductInbound::new(
            message.event_id,
            message.actor,
            message.conversation,
            payload,
        )
        .expect("normalized source message wraps as parsed inbound");
        ProductInboundEnvelope::from_trusted_parse(context, parsed)
            .expect("verified source wraps as a trusted inbound envelope")
    }

    fn assert_real_source_normalizes(self) {
        let _ = self.inbound_envelope("source-normalization");
    }
}

/// Hermetic product-facade seam for the real runner/coordinator journeys.
/// The inventory returns one exact opaque external id and routing resolves it
/// to the host-owned reply binding before writing the production outbound
/// store. Provider transport behavior remains entirely outside this double.
struct MatrixOutboundFacade {
    target: RebornOutboundDeliveryTargetOption,
    reply_target_binding_ref: ReplyTargetBindingRef,
    run_targets: Arc<dyn OutboundStateStore>,
}

impl MatrixOutboundFacade {
    fn new(
        target_id: &str,
        channel: &str,
        reply_target_binding_ref: ReplyTargetBindingRef,
        run_targets: Arc<dyn OutboundStateStore>,
    ) -> Arc<Self> {
        Arc::new(Self {
            target: RebornOutboundDeliveryTargetOption {
                target: RebornOutboundDeliveryTargetSummary::new(
                    RebornOutboundDeliveryTargetId::new(target_id)
                        .expect("matrix target id is valid"),
                    channel,
                    "Delivery matrix destination",
                    Some("Hermetic caller-owned delivery target".to_string()),
                )
                .expect("matrix target summary is valid"),
                capabilities: RebornOutboundDeliveryTargetCapabilities {
                    final_replies: true,
                    gate_prompts: false,
                    auth_prompts: false,
                },
            },
            reply_target_binding_ref,
            run_targets,
        })
    }

    async fn seal_external_run_target(
        &self,
        caller: ProductSurfaceCaller,
        run_id: ironclaw_turns::TurnRunId,
        scope: ironclaw_turns::TurnScope,
        target_id: RebornOutboundDeliveryTargetId,
    ) -> Result<(), ProductSurfaceError> {
        if scope.tenant_id != caller.tenant_id
            || scope.agent_id != caller.agent_id
            || scope.project_id != caller.project_id
            || scope
                .explicit_owner_user_id()
                .is_some_and(|owner| owner != &caller.user_id)
        {
            return Err(matrix_service_error(
                ProductSurfaceErrorCode::Forbidden,
                ProductSurfaceErrorKind::ParticipantDenied,
                403,
            ));
        }
        if target_id != self.target.target.target_id {
            return Err(matrix_service_error(
                ProductSurfaceErrorCode::NotFound,
                ProductSurfaceErrorKind::NotFound,
                404,
            ));
        }
        self.run_targets
            .put_run_final_reply_target(RunFinalReplyTargetRecord {
                run_id,
                scope,
                actor: caller.actor(),
                destination: RunFinalReplyDestination::External {
                    reply_target_binding_ref: self.reply_target_binding_ref.clone(),
                },
            })
            .await
            .map_err(|_| {
                matrix_service_error(
                    ProductSurfaceErrorCode::Unavailable,
                    ProductSurfaceErrorKind::ServiceUnavailable,
                    503,
                )
            })
    }
}

#[async_trait]
impl OutboundPreferencesProductFacade for MatrixOutboundFacade {
    async fn get_outbound_preferences(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<RebornOutboundPreferencesResponse, ProductSurfaceError> {
        Ok(RebornOutboundPreferencesResponse::default())
    }

    async fn set_outbound_preferences(
        &self,
        _caller: ProductSurfaceCaller,
        _request: RebornSetOutboundPreferencesRequest,
    ) -> Result<RebornOutboundPreferencesResponse, ProductSurfaceError> {
        Err(matrix_service_error(
            ProductSurfaceErrorCode::Unavailable,
            ProductSurfaceErrorKind::ServiceUnavailable,
            503,
        ))
    }

    async fn list_outbound_delivery_targets(
        &self,
        _caller: ProductSurfaceCaller,
    ) -> Result<RebornOutboundDeliveryTargetListResponse, ProductSurfaceError> {
        Ok(RebornOutboundDeliveryTargetListResponse {
            targets: vec![self.target.clone()],
            next_cursor: None,
        })
    }
}

fn matrix_service_error(
    code: ProductSurfaceErrorCode,
    kind: ProductSurfaceErrorKind,
    status_code: u16,
) -> ProductSurfaceError {
    ProductSurfaceError {
        code,
        kind,
        status_code,
        retryable: status_code == 503,
        field: None,
        validation_code: None,
    }
}

#[derive(Default)]
struct RecordingBotEgress {
    requests: Mutex<Vec<RestrictedEgressRequest>>,
}

impl RecordingBotEgress {
    fn requests(&self) -> Vec<RestrictedEgressRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    async fn wait_until_text_is_sent(&self, expected_text: &str) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if self.requests().iter().any(|request| {
                    String::from_utf8_lossy(request.body.as_deref().unwrap_or_default())
                        .contains(expected_text)
                }) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "bot wire did not carry {expected_text:?} before the delivery deadline: {:?}",
                self.requests()
            )
        });
    }
}

#[async_trait]
impl RestrictedEgress for RecordingBotEgress {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        let body = if request.url.contains("slack.com") {
            br#"{"ok":true,"channel":"D-MATRIX","ts":"1710000400.000100"}"#.to_vec()
        } else {
            br#"{"ok":true,"result":{"message_id":4242}}"#.to_vec()
        };
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        Ok(RestrictedEgressResponse { status: 200, body })
    }
}

#[derive(Debug)]
struct PausedReplyGateway {
    reply: String,
    release: tokio::sync::Semaphore,
}

impl PausedReplyGateway {
    fn new(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
            release: tokio::sync::Semaphore::new(0),
        }
    }

    fn release(&self) {
        self.release.add_permits(1);
    }
}

#[async_trait]
impl HostManagedModelGateway for PausedReplyGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let permit = self
            .release
            .acquire()
            .await
            .expect("paused delivery-matrix gateway remains open");
        permit.forget();
        Ok(HostManagedModelResponse::assistant_reply(&self.reply))
    }
}

struct MatrixBindingService {
    inner: Arc<dyn ConversationBindingService>,
    expected_scope: ironclaw_turns::TurnScope,
    expected_actor: ironclaw_host_api::UserId,
    source_binding: ReplyTargetBindingRef,
    source_adapter: ProductAdapterId,
    source_installation: AdapterInstallationId,
    source_conversation: ExternalConversationRef,
    destination_binding: ReplyTargetBindingRef,
    destination_adapter: ProductAdapterId,
    destination_installation: AdapterInstallationId,
    destination_conversation: ExternalConversationRef,
}

#[async_trait]
impl ConversationBindingService for MatrixBindingService {
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        self.inner.resolve_binding(request).await
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        self.inner.lookup_binding(request).await
    }

    async fn resolve_stored_reply_target(
        &self,
        request: ResolveStoredProductReplyTargetRequest,
    ) -> Result<ResolvedStoredProductReplyTarget, ProductWorkflowError> {
        if request.scope != self.expected_scope || request.actor.user_id != self.expected_actor {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        let (adapter_id, installation_id, external_conversation_ref) =
            if request.reply_target_binding_ref == self.source_binding {
                (
                    self.source_adapter.clone(),
                    self.source_installation.clone(),
                    self.source_conversation.clone(),
                )
            } else if request.reply_target_binding_ref == self.destination_binding {
                (
                    self.destination_adapter.clone(),
                    self.destination_installation.clone(),
                    self.destination_conversation.clone(),
                )
            } else {
                return Err(ProductWorkflowError::BindingAccessDenied);
            };
        Ok(ResolvedStoredProductReplyTarget {
            adapter_id,
            installation_id,
            external_conversation_ref,
            route_kind: ProductConversationRouteKind::Direct,
        })
    }
}

struct MatrixChannelResolver {
    kind: ChannelKind,
    egress: Arc<RecordingBotEgress>,
}

impl ChannelDeliveryResolver for MatrixChannelResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        (extension_id == self.kind.extension_id()).then(|| ResolvedChannelDelivery {
            extension_id: self.kind.extension_id().to_string(),
            installation_id: "delivery-matrix-installation".to_string(),
            adapter: self.kind.adapter(),
            egress: Arc::clone(&self.egress) as Arc<dyn RestrictedEgress>,
        })
    }
}

struct NoReplyContext;

#[async_trait]
impl DeliveryReplyContextSource for NoReplyContext {
    async fn reply_context(&self, _: &str, _: &str, _: &str) -> Option<Vec<u8>> {
        None
    }
}

struct ServiceableOAuthPrompt;

#[async_trait]
impl BlockedAuthPromptSource for ServiceableOAuthPrompt {
    async fn auth_prompt_for_blocked_run(
        &self,
        request: BlockedAuthPromptRequest<'_>,
    ) -> Result<AuthPromptView, ProductAdapterError> {
        Ok(AuthPromptView {
            turn_run_id: request.run_id,
            auth_request_ref: request.gate_ref.to_string(),
            invocation_id: request.invocation_id,
            headline: "Authentication required".to_string(),
            body: request.body,
            challenge_kind: Some(AuthPromptChallengeKind::OAuthUrl),
            provider: None,
            account_label: None,
            authorization_url: Some("https://auth.delivery-matrix.invalid/continue".to_string()),
            expires_at: None,
            connection: None,
            pairing: None,
        })
    }
}

struct MatrixTargetResolver {
    extension_id: String,
    expected_actor: ironclaw_host_api::UserId,
    target: ReplyTargetBindingRef,
    conversation: ExternalConversationRef,
    available: AtomicBool,
}

impl MatrixTargetResolver {
    fn remove(&self) {
        self.available.store(false, Ordering::SeqCst);
    }
}

#[async_trait]
impl CurrentDeliveryTargetResolver for MatrixTargetResolver {
    async fn resolve_current_target(
        &self,
        _scope: &ironclaw_turns::TurnScope,
        actor: &TurnActor,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<CurrentDeliveryTarget>, ProductWorkflowError> {
        if !self.available.load(Ordering::SeqCst)
            || actor.user_id != self.expected_actor
            || target != &self.target
        {
            return Ok(None);
        }
        Ok(Some(CurrentDeliveryTarget {
            extension_id: self.extension_id.clone(),
            external_conversation_ref: self.conversation.clone(),
            personal_direct_message: true,
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

struct TriggeredWireFixture {
    driver: TriggeredRunDeliveryDriver,
    event_router: Arc<RunDeliveryEventRouter>,
    egress: Arc<RecordingBotEgress>,
    outbound_store: Arc<dyn OutboundStateStore>,
    outcome_store:
        Arc<ironclaw_outbound::FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    target_resolver: Arc<MatrixTargetResolver>,
}

fn triggered_wire_fixture(
    group: &RebornIntegrationGroup,
    harness: &RebornIntegrationHarness,
    kind: ChannelKind,
    event_router: Arc<RunDeliveryEventRouter>,
    blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
) -> TriggeredWireFixture {
    let services = group
        .capability_harness()
        .expect("delivery matrix group uses the host runtime")
        .reborn_services_for_test()
        .expect("delivery matrix group exposes composed services");
    let (outbound_store, route_store, communication_preferences) = services
        .outbound_delivery_stores_for_test()
        .expect("delivery matrix uses the composed outbound stores");
    let egress = Arc::new(RecordingBotEgress::default());
    let coordinator_store = Arc::clone(&outbound_store);
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&coordinator_store),
        Arc::new(MatrixChannelResolver {
            kind,
            egress: Arc::clone(&egress),
        }),
        Arc::new(NoReplyContext),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    let fallback_notice_scope = ironclaw_turns::TurnScope::new_with_owner(
        harness.binding.tenant_id.clone(),
        harness.binding.agent_id.clone(),
        harness.binding.project_id.clone(),
        ironclaw_host_api::ThreadId::new(format!(
            "{}-delivery-matrix-notices",
            kind.extension_id()
        ))
        .expect("fallback thread id"),
        harness.binding.subject_user_id.clone(),
    );
    let run_services = RunDeliveryServices {
        binding_service: harness
            .binding_service_for_test()
            .expect("group binding service"),
        thread_service: harness
            .thread_service_for_test()
            .expect("group thread service"),
        turn_coordinator: harness.turn_coordinator_for_test(),
        outbound_store,
        route_store,
        communication_preferences,
        coordinator,
        extension_id: kind.extension_id().to_string(),
        fallback_notice_scope,
        approval_context: None,
        blocked_auth_prompts,
        auth_flow_cancel: None,
    };
    let (target, conversation) = kind.target();
    let target_resolver = Arc::new(MatrixTargetResolver {
        extension_id: kind.extension_id().to_string(),
        expected_actor: harness.binding.actor_user_id.clone(),
        target,
        conversation,
        available: AtomicBool::new(true),
    });
    let outcome_store =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let driver = TriggeredRunDeliveryDriver::with_event_router(
        run_services,
        Arc::clone(&outcome_store) as Arc<dyn TriggeredRunDeliveryStore>,
        Arc::clone(&target_resolver) as Arc<dyn CurrentDeliveryTargetResolver>,
        harness
            .binding
            .agent_id
            .clone()
            .expect("delivery matrix binding has an agent"),
        Arc::clone(&event_router),
    );
    TriggeredWireFixture {
        driver,
        event_router,
        egress,
        outbound_store: coordinator_store,
        outcome_store,
        target_resolver,
    }
}

fn trigger_context(label: &str) -> TriggerCommunicationContext {
    TriggerCommunicationContext {
        trigger_origin_ref: TriggerOriginRef::new(format!("trigger:{label}"))
            .expect("trigger origin"),
        trigger_source_kind: TriggerSourceKind::Schedule,
        fire_slot: TriggerFireSlot::new("2026-07-22T12:00:00Z").expect("trigger fire slot"),
    }
}

async fn drive_scheduled_result_to_bot_wire(
    source: ChannelKind,
    destination: ChannelKind,
    label: &str,
) -> (
    Vec<RestrictedEgressRequest>,
    TriggeredRunDeliveryOutcomeKind,
) {
    source.assert_real_source_normalizes();
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let harness = group
        .thread(format!("delivery-matrix-{label}"))
        .script([RebornScriptedReply::text(format!(
            "scheduled result for {label}"
        ))])
        .build()
        .await
        .expect("scheduled delivery thread builds");
    let submission = harness
        .submit_triggered_turn_scripted(
            format!("scheduled source request for {label}").as_str(),
            [RebornScriptedReply::text(format!(
                "scheduled result for {label}"
            ))],
        )
        .await
        .expect("real scheduled-origin run submits");
    harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::Completed,
        )
        .await
        .expect("scheduled-origin run completes");

    let event_router = Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test());
    let fixture = triggered_wire_fixture(&group, &harness, destination, event_router, None);
    let (target, _) = destination.target();
    fixture
        .driver
        .on_trigger_submitted(TriggeredRunDeliveryRequest {
            run_id: submission.run_id,
            scope: submission.turn_scope.clone(),
            creator_user_id: harness.binding.actor_user_id.clone(),
            project_scoped: false,
            prompt: format!("scheduled source request for {label}"),
            delivery_target: Some(target),
            trigger_context: trigger_context(label),
        })
        .await;
    fixture
        .event_router
        .wait_until_run_idle(submission.run_id)
        .await;
    let outcome = fixture
        .outcome_store
        .load_triggered_run_delivery(submission.run_id)
        .await
        .expect("triggered delivery outcome reads")
        .expect("triggered delivery records a terminal outcome")
        .outcome;
    let attempts = fixture
        .outbound_store
        .list_delivery_attempts(submission.turn_scope)
        .await
        .expect("scheduled coordinator attempts remain readable");
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt.status == OutboundDeliveryStatus::Delivered),
        "scheduled result must settle through the delivery coordinator; got statuses {:?}",
        attempts
            .iter()
            .map(|attempt| attempt.status)
            .collect::<Vec<_>>()
    );
    assert_only_host_delivery_operations(&harness, &[]).await;
    (fixture.egress.requests(), outcome)
}

async fn drive_scheduled_target_denial(
    label: &str,
    foreign_creator: bool,
    remove_before_fire: bool,
) -> (
    Vec<RestrictedEgressRequest>,
    TriggeredRunDeliveryOutcomeKind,
) {
    ChannelKind::Slack.assert_real_source_normalizes();
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let harness = group
        .thread(format!("delivery-matrix-{label}"))
        .script([RebornScriptedReply::text("unused thread reply")])
        .build()
        .await
        .expect("scheduled denial thread builds");
    let submission = harness
        .submit_triggered_turn_scripted(
            "scheduled result with revalidated target",
            [RebornScriptedReply::text("must not cross the bot wire")],
        )
        .await
        .expect("real scheduled-origin run submits");
    harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::Completed,
        )
        .await
        .expect("scheduled-origin run completes");

    let fixture = triggered_wire_fixture(
        &group,
        &harness,
        ChannelKind::Slack,
        Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test()),
        None,
    );
    if remove_before_fire {
        fixture.target_resolver.remove();
    }
    let (target, _) = ChannelKind::Slack.target();
    let creator_user_id = if foreign_creator {
        ironclaw_host_api::UserId::new("delivery-matrix-foreign-user").expect("foreign user id")
    } else {
        harness.binding.actor_user_id.clone()
    };
    fixture
        .driver
        .on_trigger_submitted(TriggeredRunDeliveryRequest {
            run_id: submission.run_id,
            scope: submission.turn_scope.clone(),
            creator_user_id,
            project_scoped: false,
            prompt: "scheduled result with revalidated target".to_string(),
            delivery_target: Some(target),
            trigger_context: trigger_context(label),
        })
        .await;
    fixture
        .event_router
        .wait_until_run_idle(submission.run_id)
        .await;
    let outcome = fixture
        .outcome_store
        .load_triggered_run_delivery(submission.run_id)
        .await
        .expect("triggered denial outcome reads")
        .expect("triggered denial records a terminal outcome")
        .outcome;
    let attempts = fixture
        .outbound_store
        .list_delivery_attempts(submission.turn_scope)
        .await
        .expect("denied target coordinator attempts remain readable");
    assert!(
        !attempts.is_empty()
            && attempts.iter().all(|attempt| {
                attempt.status == OutboundDeliveryStatus::Failed
                    && attempt.failure_kind == Some(DeliveryFailureKind::AuthorizationRevoked)
            }),
        "authority denial must settle only as Failed/AuthorizationRevoked and never fall back: {attempts:?}"
    );
    assert_only_host_delivery_operations(&harness, &[]).await;
    (fixture.egress.requests(), outcome)
}

fn assert_bot_wire(
    destination: ChannelKind,
    requests: &[RestrictedEgressRequest],
    expected_text: &str,
) {
    let request = requests
        .iter()
        .find(|request| {
            let body = request.body.as_deref().unwrap_or_default();
            String::from_utf8_lossy(body).contains(expected_text)
        })
        .unwrap_or_else(|| panic!("bot wire did not carry {expected_text:?}: {requests:?}"));
    match destination {
        ChannelKind::Slack => {
            assert!(
                request.url.ends_with("/api/chat.postMessage"),
                "Slack bot delivery uses chat.postMessage: {}",
                request.url
            );
            assert!(
                String::from_utf8_lossy(request.body.as_deref().unwrap_or_default())
                    .contains("\"channel\":\"D-MATRIX\""),
                "Slack bot delivery must use the selected target conversation: {request:?}"
            );
        }
        ChannelKind::Telegram => {
            assert!(
                request.url.ends_with("/sendMessage"),
                "Telegram bot delivery uses sendMessage: {}",
                request.url
            );
            assert!(
                String::from_utf8_lossy(request.body.as_deref().unwrap_or_default())
                    .contains("\"chat_id\":\"8675309\""),
                "Telegram bot delivery must use the selected target conversation: {request:?}"
            );
        }
    }
}

async fn drive_immediate_cross_channel_result(
    source: ChannelKind,
    destination: ChannelKind,
    label: &str,
) {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let harness = group
        .thread(format!("delivery-matrix-immediate-{label}"))
        .script([RebornScriptedReply::text("unused thread reply")])
        .build()
        .await
        .expect("immediate delivery thread builds");
    let services = group
        .capability_harness()
        .expect("delivery matrix group uses the host runtime")
        .reborn_services_for_test()
        .expect("delivery matrix group exposes composed services");
    let envelope = source.inbound_envelope(label);
    let source_adapter = envelope.adapter_id().clone();
    let source_installation = envelope.installation_id().clone();
    let source_conversation = envelope.external_conversation_ref().clone();
    let binding_service = harness
        .binding_service_for_test()
        .expect("group binding service");
    let source_binding = binding_service
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect("real source envelope resolves a durable binding");
    harness
        .disable_auto_approve()
        .await
        .expect("cross-channel routing must exercise the real approval gate");
    let scope = ironclaw_turns::TurnScope::new_with_owner(
        source_binding.tenant_id.clone(),
        source_binding.agent_id.clone(),
        source_binding.project_id.clone(),
        source_binding.thread_id.clone(),
        source_binding.subject_user_id.clone(),
    );
    let reply = format!("one-off explicit result for {label}");
    let (destination_binding, destination_conversation) = destination.target();
    let opaque_target_id = format!("opaque-delivery-matrix-{label}");
    group
        .register_source_delivery_target_for_test(
            format!("delivery-matrix-{label}-target").as_str(),
            &opaque_target_id,
            destination_binding.clone(),
        )
        .expect("caller-owned destination target registers");
    let (outbound_store, route_store, communication_preferences) = services
        .outbound_delivery_stores_for_test()
        .expect("delivery matrix uses the composed outbound stores");
    let egress = Arc::new(RecordingBotEgress::default());
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&outbound_store),
        Arc::new(MatrixChannelResolver {
            kind: destination,
            egress: Arc::clone(&egress),
        }),
        Arc::new(NoReplyContext),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    let matrix_binding_service = Arc::new(MatrixBindingService {
        inner: binding_service,
        expected_scope: scope.clone(),
        expected_actor: source_binding.actor_user_id.clone(),
        source_binding: source_binding.reply_target_binding_ref,
        source_adapter: source_adapter.clone(),
        source_installation: source_installation.clone(),
        source_conversation,
        destination_binding: destination_binding.clone(),
        destination_adapter: ProductAdapterId::new(destination.extension_id())
            .expect("destination adapter id"),
        destination_installation: AdapterInstallationId::new("delivery-matrix-installation")
            .expect("destination installation id"),
        destination_conversation: destination_conversation.clone(),
    });
    let current_target_resolver = Arc::new(MatrixTargetResolver {
        extension_id: destination.extension_id().to_string(),
        expected_actor: source_binding.actor_user_id.clone(),
        target: destination_binding.clone(),
        conversation: destination_conversation,
        available: AtomicBool::new(true),
    });
    let run_services = RunDeliveryServices {
        binding_service: matrix_binding_service,
        thread_service: harness
            .thread_service_for_test()
            .expect("group thread service"),
        turn_coordinator: harness.turn_coordinator_for_test(),
        outbound_store: Arc::clone(&outbound_store),
        route_store,
        communication_preferences,
        coordinator,
        extension_id: destination.extension_id().to_string(),
        fallback_notice_scope: scope.clone(),
        approval_context: None,
        blocked_auth_prompts: None,
        auth_flow_cancel: None,
    };
    let event_router = group
        .run_delivery_events()
        .expect("delivery group wires the lifecycle router");
    let handler = Arc::new(
        RunDeliveryEventHandler::new(
            run_services,
            destination.extension_id(),
            "delivery-matrix-installation",
        )
        .with_current_target_resolver(current_target_resolver),
    );
    event_router.register(destination.extension_id(), &handler);

    group
        .register_scope_script_for_test(
            scope.clone(),
            format!("delivery-matrix-channel-source-{label}").as_str(),
            [
                RebornScriptedReply::tool_call(
                    ROUTE_CURRENT,
                    json!({"target_id": opaque_target_id}),
                ),
                RebornScriptedReply::text(reply.clone()),
            ],
        )
        .await
        .expect("channel source receives a real scripted model gateway");
    let ack = harness
        .product_workflow_for_test()
        .submit_inbound(envelope)
        .await
        .expect("real source envelope is admitted");
    let ProductInboundAck::Accepted {
        submitted_run_id: run_id,
        ..
    } = ack
    else {
        panic!("real source envelope must submit one run: {ack:?}");
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = harness
                .turn_coordinator_for_test()
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await
                .expect("cross-channel run remains readable");
            match state.status {
                TurnStatus::BlockedApproval => {
                    let gate_ref = state
                        .gate_ref
                        .expect("blocked current-run routing carries an approval gate ref");
                    harness
                        .approve_gate_in_scope(&scope, run_id, &gate_ref)
                        .await
                        .expect("the caller can approve the exact blocked routing operation");
                    return;
                }
                TurnStatus::Completed => return,
                TurnStatus::Failed | TurnStatus::Cancelled => {
                    panic!(
                        "cross-channel routing reached terminal status {:?}: {:?}",
                        state.status, state.failure
                    );
                }
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("cross-channel route either blocks for approval or completes");
    harness
        .wait_for_status_in_scope(&scope, run_id, TurnStatus::Completed)
        .await
        .expect("the same source run completes after routing approval");
    event_router.wait_until_run_idle(run_id).await;
    event_router.wait_until_durable_replay_idle().await;

    harness
        .assert_tool_invoked(ROUTE_CURRENT)
        .await
        .expect("the channel-origin run dispatches the normal first-party route capability");
    let selected_record = outbound_store
        .load_run_final_reply_target(RunFinalReplyTargetRequest {
            run_id,
            scope: scope.clone(),
            actor: TurnActor::new(source_binding.actor_user_id.clone()),
        })
        .await
        .expect("the sealed run target remains readable")
        .expect("the first-party route handler writes one exact run target");
    assert_eq!(
        selected_record.destination,
        RunFinalReplyDestination::External {
            reply_target_binding_ref: destination_binding.clone(),
        },
        "the route must preserve the host-resolved binding behind the opaque registry id"
    );

    // #6520 delivery is event-driven; under instrumented (coverage) builds the
    // send can land after the router reports idle. Poll the wire with the same
    // bounded deadline the file's other async seams use before asserting.
    let wire_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let destination_requests = loop {
        let requests = egress.requests();
        if requests.iter().any(|request| {
            String::from_utf8_lossy(request.body.as_deref().unwrap_or_default()).contains(&reply)
        }) {
            break requests;
        }
        assert!(
            tokio::time::Instant::now() < wire_deadline,
            "bot wire did not carry {reply:?}: {requests:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_bot_wire(destination, &destination_requests, &reply);
    assert_eq!(
        destination_requests
            .iter()
            .filter(|request| {
                String::from_utf8_lossy(request.body.as_deref().unwrap_or_default())
                    .contains(&reply)
            })
            .count(),
        1,
        "the selected bot wire must receive the one-off result exactly once"
    );
    assert!(
        destination_requests.iter().all(|request| {
            !String::from_utf8_lossy(request.body.as_deref().unwrap_or_default())
                .contains("Ironclaw is thinking...")
        }),
        "the explicit destination receives only the result, never the source placeholder"
    );

    let source_requests = harness.captured_network_requests_for_test();
    let (source_send_suffix, source_retract_suffix) = match source {
        ChannelKind::Slack => ("/api/chat.postMessage", "/api/chat.delete"),
        ChannelKind::Telegram => ("/sendMessage", "/deleteMessage"),
    };
    let source_sends: Vec<_> = source_requests
        .iter()
        .filter(|request| request.url.ends_with(source_send_suffix))
        .collect();
    let source_retracts = source_requests
        .iter()
        .filter(|request| request.url.ends_with(source_retract_suffix))
        .count();
    assert!(
        source_sends
            .iter()
            .all(|request| !String::from_utf8_lossy(&request.body).contains(&reply)),
        "the explicit cross-channel result must not be duplicated back to its source"
    );
    assert_eq!(
        source_retracts,
        source_sends.len(),
        "every source working/gate placeholder must be retracted after the final result routes elsewhere: {source_requests:?}"
    );
    let attempts = outbound_store
        .list_delivery_attempts(scope)
        .await
        .expect("immediate coordinator attempts remain readable");
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt.status == OutboundDeliveryStatus::Delivered),
        "one-off cross-channel result must settle through the coordinator; got {attempts:?}"
    );
    assert_only_host_delivery_operations(&harness, &[ROUTE_CURRENT]).await;
}

async fn build_target_listing_harness(
    group: &RebornIntegrationGroup,
    conversation_id: &str,
) -> RebornIntegrationHarness {
    group
        .thread(conversation_id)
        .script([
            RebornScriptedReply::tool_call(TARGETS_LIST, json!({})),
            RebornScriptedReply::text("I found the available result destinations."),
        ])
        .build()
        .await
        .expect("delivery-routing thread builds")
}

async fn list_targets(harness: &RebornIntegrationHarness) -> Vec<Value> {
    harness
        .submit_turn("Where can you send this result?")
        .await
        .expect("target-list turn completes");
    harness
        .assert_tool_invoked(TARGETS_LIST)
        .await
        .expect("target inventory is read through the model-facing operation");
    harness
        .tool_result_output(TARGETS_LIST)
        .await
        .expect("target-list result is recorded")["targets"]
        .as_array()
        .expect("target-list result has a targets array")
        .clone()
}

fn listed_target_id(target: &Value) -> String {
    target["target"]["target_id"]
        .as_str()
        .expect("listed target carries an opaque target_id")
        .to_string()
}

fn is_final_reply_target(target: &&Value) -> bool {
    target["capabilities"]["final_replies"] == json!(true)
}

async fn assert_only_host_delivery_operations(
    harness: &RebornIntegrationHarness,
    allowed: &[&str],
) {
    harness
        .assert_only_tools_invoked(allowed)
        .await
        .expect(
            "delivery must stay on host-owned operations, without the user-wide setter or any provider send-as-user tool",
        );
}

#[tokio::test]
async fn explicit_telegram_source_immediate_result_delivers_to_slack_bot_wire() {
    drive_immediate_cross_channel_result(
        ChannelKind::Telegram,
        ChannelKind::Slack,
        "telegram-source-to-slack-immediate",
    )
    .await;
}

#[tokio::test]
async fn explicit_slack_source_immediate_result_delivers_to_telegram_bot_wire() {
    drive_immediate_cross_channel_result(
        ChannelKind::Slack,
        ChannelKind::Telegram,
        "slack-source-to-telegram-immediate",
    )
    .await;
}

#[tokio::test]
async fn immediate_external_result_uses_exact_listed_target_without_changing_default() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound target group builds");
    let harness = build_target_listing_harness(&group, "delivery-route-current-external").await;
    let targets = list_targets(&harness).await;
    let external = targets
        .iter()
        .filter(is_final_reply_target)
        .find(|target| target["target"]["channel"] != json!("web_app"))
        .expect("inventory contains an external final-reply destination");
    let exact_listed_id = listed_target_id(external);
    let expected_reply_target = ReplyTargetBindingRef::new("reply:route-current-external")
        .expect("route-current external reply target");
    group
        .register_source_delivery_target_for_test(
            "route-current-external",
            &exact_listed_id,
            expected_reply_target.clone(),
        )
        .expect("listed external target registers in production target authority");

    harness.push_script([
        RebornScriptedReply::tool_call(
            ROUTE_CURRENT,
            json!({"target_id": exact_listed_id.clone()}),
        ),
        RebornScriptedReply::text("one-off routed result"),
    ]);
    let routed_run_id = harness
        .submit_turn("Send only this result to that destination.")
        .await
        .expect("run-scoped route completes");

    harness
        .assert_tool_invoked(ROUTE_CURRENT)
        .await
        .expect("the run-scoped routing operation is dispatched");
    let routed = harness
        .tool_result_output(ROUTE_CURRENT)
        .await
        .expect("the run-scoped route records a capability result");
    assert_eq!(
        routed["routed"],
        json!(true),
        "the exact listed target is accepted as this run's route"
    );
    assert_only_host_delivery_operations(&harness, &[TARGETS_LIST, ROUTE_CURRENT]).await;
    let services = group
        .capability_harness()
        .expect("outbound target group uses the host runtime")
        .reborn_services_for_test()
        .expect("outbound target group exposes composed services");
    let (outbound_store, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("composed services expose the outbound route store");
    let route = outbound_store
        .load_run_final_reply_target(RunFinalReplyTargetRequest {
            run_id: routed_run_id,
            scope: harness.turn_scope.clone(),
            actor: TurnActor::new(harness.binding.actor_user_id.clone()),
        })
        .await
        .expect("normal first-party route remains readable")
        .expect("one exact run-scoped route is recorded");
    assert_eq!(
        route.destination,
        RunFinalReplyDestination::External {
            reply_target_binding_ref: expected_reply_target,
        }
    );
    let facade = group
        .capability_harness()
        .expect("outbound target group uses the host runtime")
        .outbound_preferences_facade_for_test()
        .expect("outbound target group exposes its strict facade double");
    assert!(
        facade.recorded_set_target_ids().is_empty(),
        "routing one answer must not mutate the user-wide default"
    );
}

#[tokio::test]
async fn immediate_web_app_result_uses_exact_host_owned_target_without_external_egress() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound target group builds");
    let harness = build_target_listing_harness(&group, "delivery-route-current-web-app").await;
    let targets = list_targets(&harness).await;
    let web_app = targets
        .iter()
        .find(|target| {
            target["target"]["channel"] == json!("web_app") && is_final_reply_target(target)
        })
        .expect("inventory includes the host-owned web_app final-reply destination");
    let exact_listed_id = listed_target_id(web_app);

    harness.push_script([
        RebornScriptedReply::tool_call(
            ROUTE_CURRENT,
            json!({"target_id": exact_listed_id.clone()}),
        ),
        RebornScriptedReply::text("host-side only result"),
    ]);
    let routed_run_id = harness
        .submit_turn("Keep only this result in the web app.")
        .await
        .expect("host-owned route completes");

    harness
        .assert_tool_invoked(ROUTE_CURRENT)
        .await
        .expect("the run-scoped routing operation is dispatched");
    let routed = harness
        .tool_result_output(ROUTE_CURRENT)
        .await
        .expect("the run-scoped WebUI route records a capability result");
    assert_eq!(
        routed["routed"],
        json!(true),
        "the host-owned web_app target is accepted as this run's route"
    );
    assert_only_host_delivery_operations(&harness, &[TARGETS_LIST, ROUTE_CURRENT]).await;

    let services = group
        .capability_harness()
        .expect("outbound target group uses the host runtime")
        .reborn_services_for_test()
        .expect("outbound target group exposes composed services");
    let (outbound_store, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("composed services expose the outbound route store");
    let route = outbound_store
        .load_run_final_reply_target(RunFinalReplyTargetRequest {
            run_id: routed_run_id,
            scope: harness.turn_scope.clone(),
            actor: TurnActor::new(harness.binding.actor_user_id.clone()),
        })
        .await
        .expect("normal first-party WebApp route remains readable")
        .expect("one exact WebApp route is recorded");
    assert_eq!(route.destination, RunFinalReplyDestination::WebApp);
    let facade = group
        .capability_harness()
        .expect("outbound target group uses the host runtime")
        .outbound_preferences_facade_for_test()
        .expect("outbound target group exposes its strict facade double");
    assert!(
        facade.recorded_set_target_ids().is_empty(),
        "routing one answer to WebUI must not mutate the user-wide default"
    );

    let attempts = outbound_store
        .list_delivery_attempts(harness.turn_scope.clone())
        .await
        .expect("coordinator attempts remain readable");
    assert!(
        attempts.is_empty(),
        "the web_app route persists the normal run result and must not create external egress \
         attempts: {attempts:?}"
    );
}

#[tokio::test]
async fn removed_current_run_target_is_model_correctable_not_an_encoding_failure() {
    let group = RebornIntegrationGroup::outbound_target_tools()
        .await
        .expect("outbound target group builds");
    let harness = group
        .thread("delivery-route-current-removed")
        .script([
            RebornScriptedReply::tool_call(
                ROUTE_CURRENT,
                json!({"target_id": "opaque-target-removed-before-routing"}),
            ),
            RebornScriptedReply::text("That destination is no longer available."),
        ])
        .build()
        .await
        .expect("removed-target routing thread builds");

    harness
        .submit_turn("Send only this result to the removed destination.")
        .await
        .expect("recoverable target rejection leaves the turn usable");
    harness
        .assert_tool_error(ToolErrorClass::Failed, "invalid_input")
        .await
        .expect("the stale target is a structured model-correctable validation failure");
    harness
        .assert_no_tool_error(ToolErrorClass::Failed, "could not be encoded")
        .await
        .expect("target authority denial must not masquerade as argument encoding failure");
    assert_only_host_delivery_operations(&harness, &[ROUTE_CURRENT]).await;
}

#[tokio::test]
async fn web_app_source_can_seal_the_current_run_to_web_app_without_external_egress() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let gate = ParkingModelGate::new();
    let harness = group
        .thread("delivery-route-current-web-app-real-store")
        .script([RebornScriptedReply::text("host-side only result")])
        .park_model(gate.clone())
        .build()
        .await
        .expect("web app delivery thread builds");
    let run_id = harness
        .submit_turn_async("Keep only this result in the web app.")
        .await
        .expect("web app source run submits");
    gate.wait_until_parked().await;

    let services = group
        .capability_harness()
        .expect("delivery matrix group uses the host runtime")
        .reborn_services_for_test()
        .expect("delivery matrix group exposes composed services");
    let caller = ProductSurfaceCaller::new(
        harness.binding.tenant_id.clone(),
        harness.binding.actor_user_id.clone(),
        harness.binding.agent_id.clone(),
        harness.binding.project_id.clone(),
    );
    let (outbound_store, route_store, communication_preferences) = services
        .outbound_delivery_stores_for_test()
        .expect("delivery matrix uses the composed outbound stores");
    outbound_store
        .put_run_final_reply_target(RunFinalReplyTargetRecord {
            run_id,
            scope: harness.turn_scope.clone(),
            actor: caller.actor(),
            destination: RunFinalReplyDestination::WebApp,
        })
        .await
        .expect("the host-owned WebUI destination is sealed onto this run");
    let selected_record = outbound_store
        .load_run_final_reply_target(RunFinalReplyTargetRequest {
            run_id,
            scope: harness.turn_scope.clone(),
            actor: caller.actor(),
        })
        .await
        .expect("the sealed WebUI target remains readable")
        .expect("the caller's route writes one run target");
    assert_eq!(
        selected_record.destination,
        RunFinalReplyDestination::WebApp
    );

    // Register the production lifecycle consumer against the real durable
    // target store. The probe egress makes any accidental external candidate
    // observable; `WebApp` must stop before target resolution or dispatch.
    let egress = Arc::new(RecordingBotEgress::default());
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&outbound_store),
        Arc::new(MatrixChannelResolver {
            kind: ChannelKind::Slack,
            egress: Arc::clone(&egress),
        }),
        Arc::new(NoReplyContext),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    let run_services = RunDeliveryServices {
        binding_service: harness
            .binding_service_for_test()
            .expect("group binding service"),
        thread_service: harness
            .thread_service_for_test()
            .expect("group thread service"),
        turn_coordinator: harness.turn_coordinator_for_test(),
        outbound_store: Arc::clone(&outbound_store),
        route_store,
        communication_preferences,
        coordinator,
        extension_id: "web-app-no-egress-probe".to_string(),
        fallback_notice_scope: harness.turn_scope.clone(),
        approval_context: None,
        blocked_auth_prompts: None,
        auth_flow_cancel: None,
    };
    let handler = Arc::new(RunDeliveryEventHandler::new(
        run_services,
        "web-app-no-egress-probe",
        "web-app-no-egress-probe-installation",
    ));
    let event_router = group
        .run_delivery_events()
        .expect("delivery group exposes the production lifecycle router");
    event_router.register("web-app-no-egress-probe", &handler);

    gate.release();
    let completed = harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("WebUI-only run completes normally");
    event_router
        .publish(TurnLifecycleEvent::from_run_state(
            &completed,
            TurnEventKind::Completed,
            None,
        ))
        .await
        .expect("the lifecycle router accepts the completion event");
    event_router.wait_until_run_idle(run_id).await;
    let attempts = outbound_store
        .list_delivery_attempts(harness.turn_scope.clone())
        .await
        .expect("WebUI coordinator attempts remain readable");
    assert!(
        attempts.is_empty(),
        "the WebUI route stores the normal result without external coordinator attempts: {attempts:?}"
    );
    assert!(
        egress.requests().is_empty(),
        "the production lifecycle consumer must not hand a WebApp result to a channel wire"
    );
    assert_only_host_delivery_operations(&harness, &[]).await;
}

#[tokio::test]
async fn web_app_source_immediate_result_fans_out_to_selected_external_bot_wire() {
    let destination = ChannelKind::Slack;
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let harness = group
        .thread("delivery-web-app-source-to-external")
        .script([RebornScriptedReply::text(
            "WebUI-origin result routed to the selected external destination",
        )])
        .build()
        .await
        .expect("WebUI external-delivery thread builds");
    let services = group
        .capability_harness()
        .expect("delivery matrix group uses the host runtime")
        .reborn_services_for_test()
        .expect("delivery matrix group exposes composed services");
    let (destination_binding, destination_conversation) = destination.target();
    let opaque_target_id = "opaque-web-app-source-external-target";
    group
        .register_source_delivery_target_for_test(
            "delivery-web-app-source-external-target",
            opaque_target_id,
            destination_binding.clone(),
        )
        .expect("caller-owned external destination registers");
    let (outbound_store, route_store, communication_preferences) = services
        .outbound_delivery_stores_for_test()
        .expect("delivery matrix uses the composed outbound stores");
    let outbound_facade = MatrixOutboundFacade::new(
        opaque_target_id,
        destination.extension_id(),
        destination_binding.clone(),
        Arc::clone(&outbound_store),
    );
    let caller = ProductSurfaceCaller::new(
        harness.binding.tenant_id.clone(),
        harness.binding.actor_user_id.clone(),
        harness.binding.agent_id.clone(),
        harness.binding.project_id.clone(),
    );

    let webui = RebornServices::new(
        harness
            .thread_service_for_test()
            .expect("group thread service"),
        harness.turn_coordinator_for_test(),
    )
    .with_outbound_preferences_facade(
        Arc::clone(&outbound_facade) as Arc<dyn OutboundPreferencesProductFacade>
    );
    let created = webui
        .create_thread(
            caller.clone(),
            ProductCreateThreadRequest {
                client_action_id: Some("delivery-web-app-source-create".to_string()),
                requested_thread_id: Some("delivery-web-app-source-external".to_string()),
                project_id: None,
            },
        )
        .await
        .expect("the actual WebUI product API creates its caller-owned thread");
    let webui_scope = caller.turn_scope(created.thread.thread_id.clone());
    let gateway = Arc::new(PausedReplyGateway::new(
        "WebUI-origin result routed to the selected external destination",
    ));
    harness.register_scope_gateway_for_test(
        webui_scope.clone(),
        Arc::clone(&gateway) as Arc<dyn HostManagedModelGateway>,
    );
    let target_page = webui
        .query(
            caller.clone(),
            RebornViewQuery {
                view_id: OUTBOUND_DELIVERY_TARGETS_VIEW.id.to_string(),
                params: json!({}),
                cursor: None,
            },
        )
        .await
        .expect("WebUI caller target inventory resolves");
    let target_inventory: RebornOutboundDeliveryTargetListResponse =
        serde_json::from_value(target_page.payload)
            .expect("outbound target view carries its typed response");
    let selected = target_inventory
        .targets
        .into_iter()
        .find(|target| {
            target.capabilities.final_replies
                && target.target.channel.as_str() == destination.extension_id()
        })
        .expect("the external final-reply target is listed")
        .target
        .target_id;
    let submitted = webui
        .submit_turn(
            caller.clone(),
            ProductSubmitTurnRequest {
                client_action_id: Some("delivery-web-app-source-external-action".to_string()),
                thread_id: Some(created.thread.thread_id.as_str().to_string()),
                content: Some("Send this one result to my selected external destination.".into()),
                ..ProductSubmitTurnRequest::default()
            },
        )
        .await
        .expect("the actual WebUI product API submits a turn");
    let RebornSubmitTurnResponse::Submitted { run_id, .. } = submitted else {
        panic!("WebUI source must submit one run: {submitted:?}");
    };
    outbound_facade
        .seal_external_run_target(
            caller.clone(),
            run_id,
            webui_scope.clone(),
            selected.clone(),
        )
        .await
        .expect("the exact listed external target is sealed onto the WebUI run");

    let selected_record = outbound_store
        .load_run_final_reply_target(RunFinalReplyTargetRequest {
            run_id,
            scope: webui_scope.clone(),
            actor: caller.actor(),
        })
        .await
        .expect("the WebUI-origin run target remains readable")
        .expect("the caller's exact external route writes one run target");
    assert_eq!(
        selected_record.destination,
        RunFinalReplyDestination::External {
            reply_target_binding_ref: destination_binding.clone(),
        },
        "the WebUI route must preserve the host-resolved binding behind the exact listed id"
    );
    let egress = Arc::new(RecordingBotEgress::default());
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&outbound_store),
        Arc::new(MatrixChannelResolver {
            kind: destination,
            egress: Arc::clone(&egress),
        }),
        Arc::new(NoReplyContext),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    let run_services = RunDeliveryServices {
        binding_service: harness
            .binding_service_for_test()
            .expect("group binding service"),
        thread_service: harness
            .thread_service_for_test()
            .expect("group thread service"),
        turn_coordinator: harness.turn_coordinator_for_test(),
        outbound_store: Arc::clone(&outbound_store),
        route_store,
        communication_preferences,
        coordinator,
        extension_id: destination.extension_id().to_string(),
        fallback_notice_scope: webui_scope.clone(),
        approval_context: None,
        blocked_auth_prompts: None,
        auth_flow_cancel: None,
    };
    let current_target_resolver = Arc::new(MatrixTargetResolver {
        extension_id: destination.extension_id().to_string(),
        expected_actor: harness.binding.actor_user_id.clone(),
        target: destination_binding,
        conversation: destination_conversation,
        available: AtomicBool::new(true),
    });
    let handler = Arc::new(
        RunDeliveryEventHandler::new(
            run_services,
            destination.extension_id(),
            "delivery-matrix-installation",
        )
        .with_current_target_resolver(current_target_resolver),
    );
    let event_router = group
        .run_delivery_events()
        .expect("delivery group wires the lifecycle router");
    event_router.register(destination.extension_id(), &handler);

    gateway.release();
    let completed = harness
        .wait_for_status_in_scope(&webui_scope, run_id, TurnStatus::Completed)
        .await
        .expect("WebUI-origin run completes");
    let product_context = completed
        .product_context
        .expect("WebUI run carries typed product context");
    assert_eq!(product_context.origin, TurnOriginKind::WebUi);
    assert!(
        product_context.adapter.is_none(),
        "WebUI product context must remain provider-neutral"
    );
    egress
        .wait_until_text_is_sent("WebUI-origin result routed to the selected external destination")
        .await;
    event_router.wait_until_run_idle(run_id).await;

    assert_bot_wire(
        destination,
        &egress.requests(),
        "WebUI-origin result routed to the selected external destination",
    );
    let attempts = outbound_store
        .list_delivery_attempts(webui_scope)
        .await
        .expect("WebUI-to-external coordinator attempts remain readable");
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt.status == OutboundDeliveryStatus::Delivered),
        "WebUI-origin external result must settle through the coordinator: {attempts:?}"
    );
    assert_only_host_delivery_operations(&harness, &[]).await;
}

#[tokio::test]
async fn explicit_slack_source_scheduled_fire_delivers_to_slack_bot_wire() {
    let (requests, outcome) = drive_scheduled_result_to_bot_wire(
        ChannelKind::Slack,
        ChannelKind::Slack,
        "explicit-slack-source-scheduled",
    )
    .await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    assert_bot_wire(ChannelKind::Slack, &requests, "scheduled result");
}

#[tokio::test]
async fn explicit_telegram_source_scheduled_fire_delivers_to_telegram_bot_wire() {
    let (requests, outcome) = drive_scheduled_result_to_bot_wire(
        ChannelKind::Telegram,
        ChannelKind::Telegram,
        "explicit-telegram-source-scheduled",
    )
    .await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    assert_bot_wire(ChannelKind::Telegram, &requests, "scheduled result");
}

#[tokio::test]
async fn explicit_telegram_source_scheduled_fire_delivers_to_slack_bot_wire() {
    let (requests, outcome) = drive_scheduled_result_to_bot_wire(
        ChannelKind::Telegram,
        ChannelKind::Slack,
        "telegram-source-to-slack-scheduled",
    )
    .await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    assert_bot_wire(ChannelKind::Slack, &requests, "scheduled result");
}

#[tokio::test]
async fn explicit_slack_source_scheduled_fire_delivers_to_telegram_bot_wire() {
    let (requests, outcome) = drive_scheduled_result_to_bot_wire(
        ChannelKind::Slack,
        ChannelKind::Telegram,
        "slack-source-to-telegram-scheduled",
    )
    .await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Delivered);
    assert_bot_wire(ChannelKind::Telegram, &requests, "scheduled result");
}

#[tokio::test]
async fn blocked_auth_resume_preserves_the_selected_run_target() {
    ChannelKind::Telegram.assert_real_source_normalizes();
    let group = RebornIntegrationGroup::live_auth_and_approval()
        .await
        .expect("auth-capable delivery group builds");
    let harness = group
        .thread("delivery-matrix-blocked-auth-resume")
        .script([RebornScriptedReply::text("unused thread reply")])
        .build()
        .await
        .expect("blocked-auth delivery thread builds");
    harness
        .enable_auto_approve()
        .await
        .expect("the selected journey isolates the auth gate");
    let submission = harness
        .submit_triggered_turn_scripted(
            "scheduled result must retain its selected Slack target across auth",
            [
                RebornScriptedReply::tool_call(
                    "github.get_repo",
                    json!({"owner": "octocat", "repo": "hello-world"}),
                ),
                RebornScriptedReply::text("selected-target result after the auth gate was denied"),
            ],
        )
        .await
        .expect("real scheduled-origin auth-gated run submits");
    let blocked = harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedAuth,
        )
        .await
        .expect("real capability dispatch parks on the auth gate");
    let gate_ref = blocked
        .gate_ref
        .expect("blocked auth run carries a gate ref");

    let fixture = triggered_wire_fixture(
        &group,
        &harness,
        ChannelKind::Slack,
        Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test()),
        Some(Arc::new(ServiceableOAuthPrompt)),
    );
    let (selected_target, _) = ChannelKind::Slack.target();
    fixture
        .driver
        .communication_preferences()
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(
                    submission.turn_scope.tenant_id.clone(),
                    harness.binding.actor_user_id.clone(),
                ),
                final_reply_target: Some(selected_target.clone()),
                progress_target: None,
                approval_prompt_target: None,
                auth_prompt_target: Some(selected_target.clone()),
                default_modality: Some(CommunicationModality::Text),
                updated_at: Utc::now(),
                updated_by: harness.binding.actor_user_id.clone(),
            },
            expected_version: None,
        })
        .await
        .expect("the caller's source channel is registered for auth prompts");
    fixture
        .driver
        .on_trigger_submitted(TriggeredRunDeliveryRequest {
            run_id: submission.run_id,
            scope: submission.turn_scope.clone(),
            creator_user_id: harness.binding.actor_user_id.clone(),
            project_scoped: false,
            prompt: "scheduled result must retain its selected Slack target across auth"
                .to_string(),
            delivery_target: Some(selected_target),
            trigger_context: trigger_context("blocked-auth-selected-target"),
        })
        .await;
    fixture
        .event_router
        .wait_until_run_idle(submission.run_id)
        .await;
    let prompt_requests = fixture.egress.requests();
    let prompt_attempts = fixture
        .outbound_store
        .list_delivery_attempts(submission.turn_scope.clone())
        .await
        .expect("blocked-auth prompt attempts remain readable");
    let prompt_outcome = fixture
        .outcome_store
        .load_triggered_run_delivery(submission.run_id)
        .await
        .expect("blocked-auth prompt outcome remains readable");
    assert!(
        !prompt_requests.is_empty(),
        "serviceable auth prompt must reach the source channel; attempts={prompt_attempts:?}, outcome={prompt_outcome:?}"
    );
    assert_bot_wire(
        ChannelKind::Slack,
        &prompt_requests,
        "auth.delivery-matrix.invalid",
    );

    harness
        .resume_run_in_scope_impl(
            submission.turn_scope.clone(),
            submission.run_id,
            gate_ref,
            Some(GateResumeDisposition::Denied),
            ResumeTurnPrecondition::BlockedAuthGate,
        )
        .await
        .expect("denying the auth challenge resumes the same triggered run");
    let completed = harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::Completed,
        )
        .await
        .expect("denied auth is surfaced to the model and the run completes");
    fixture
        .event_router
        .publish(TurnLifecycleEvent::from_run_state(
            &completed,
            TurnEventKind::Completed,
            None,
        ))
        .await
        .expect("completed lifecycle event publishes to the retained delivery handler");
    fixture
        .event_router
        .wait_until_run_idle(submission.run_id)
        .await;

    let requests = fixture.egress.requests();
    assert_bot_wire(
        ChannelKind::Slack,
        &requests,
        "selected-target result after the auth gate was denied",
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.ends_with("/api/chat.postMessage"))
            .count(),
        2,
        "the auth prompt and resumed final reply both use the selected Slack bot wire"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.ends_with("/api/chat.delete"))
            .count(),
        1,
        "the delivered auth prompt is retracted exactly once before the final reply"
    );
    let attempts = fixture
        .outbound_store
        .list_delivery_attempts(submission.turn_scope)
        .await
        .expect("blocked-auth coordinator attempts remain readable");
    assert_eq!(
        attempts
            .iter()
            .filter(|attempt| attempt.status == OutboundDeliveryStatus::Delivered)
            .count(),
        3,
        "the auth prompt, its one-time cleanup, and resumed final reply all settle through the coordinator"
    );
    assert_only_host_delivery_operations(&harness, &["github.get_repo"]).await;
}

#[tokio::test]
async fn foreign_user_scheduled_target_fails_closed_before_bot_wire() {
    let (requests, outcome) =
        drive_scheduled_target_denial("foreign-user-target", true, false).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Denied);
    assert!(
        requests.is_empty(),
        "foreign-user authority denial must happen before vendor egress: {requests:?}"
    );
}

#[tokio::test]
async fn removed_scheduled_target_fails_closed_at_fire_time_before_bot_wire() {
    let (requests, outcome) =
        drive_scheduled_target_denial("removed-target-at-fire", false, true).await;
    assert_eq!(outcome, TriggeredRunDeliveryOutcomeKind::Denied);
    assert!(
        requests.is_empty(),
        "removed-target authority denial must happen before vendor egress: {requests:?}"
    );
}

async fn registered_scheduled_target(
    group: &RebornIntegrationGroup,
    harness: &RebornIntegrationHarness,
    provider_key: &str,
    target_id: &str,
) -> (String, ReplyTargetBindingRef) {
    let source_run_id = harness
        .submit_turn("Establish this caller-owned source conversation.")
        .await
        .expect("source conversation turn completes");
    let source_run = harness
        .turn_state_store_for_test()
        .get_run_state(GetRunStateRequest {
            scope: harness.turn_scope.clone(),
            run_id: source_run_id,
        })
        .await
        .expect("source run remains readable");
    let reply_target_binding_ref = source_run.reply_target_binding_ref;
    group
        .register_source_delivery_target_for_test(
            provider_key,
            target_id,
            reply_target_binding_ref.clone(),
        )
        .expect("caller-owned target registers on the real registry");

    let services = group
        .capability_harness()
        .expect("trigger group uses the host runtime")
        .reborn_services_for_test()
        .expect("trigger group exposes composed services");
    let caller = ProductSurfaceCaller::new(
        harness.binding.tenant_id.clone(),
        harness.binding.actor_user_id.clone(),
        harness.binding.agent_id.clone(),
        harness.binding.project_id.clone(),
    );
    let (outbound_store, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("trigger group exposes the composed outbound store");
    let outbound_facade = MatrixOutboundFacade::new(
        target_id,
        "test-channel",
        reply_target_binding_ref.clone(),
        outbound_store,
    );
    let inventory = outbound_facade
        .list_outbound_delivery_targets(caller)
        .await
        .expect("caller-scoped target inventory resolves");
    let only_target = inventory
        .targets
        .iter()
        .find(|target| {
            target.capabilities.final_replies && target.target.target_id.as_str() == target_id
        })
        .expect("registered final-reply target is listed");
    (
        only_target.target.target_id.as_str().to_string(),
        reply_target_binding_ref,
    )
}

#[tokio::test]
async fn scheduled_routine_persists_the_exact_listed_target() {
    let group = RebornIntegrationGroup::triggers()
        .await
        .expect("trigger group builds");
    let harness = group
        .thread("delivery-scheduled-explicit")
        .script([RebornScriptedReply::text("source established")])
        .build()
        .await
        .expect("trigger thread builds");
    let (exact_listed_id, _) = registered_scheduled_target(
        &group,
        &harness,
        "delivery-scheduled-explicit-provider",
        "opaque-scheduled-target",
    )
    .await;

    harness.push_script([
        RebornScriptedReply::tool_call(
            TRIGGER_CREATE,
            json!({
                "name": "explicit listed delivery target",
                "prompt": "send the scheduled result",
                "schedule": {
                    "kind": "once",
                    "at": "2999-01-01T00:00:00",
                    "timezone": "UTC"
                },
                "delivery_target_id": exact_listed_id,
            }),
        ),
        RebornScriptedReply::text("scheduled"),
    ]);
    harness
        .submit_turn("Schedule that result to the destination you just listed.")
        .await
        .expect("explicitly routed trigger creation completes");
    let output = harness
        .tool_result_output(TRIGGER_CREATE)
        .await
        .expect("trigger create result is recorded");
    assert_eq!(
        output["trigger"]["delivery_target_id"],
        json!(exact_listed_id),
        "the routine must persist the exact opaque id returned by target listing"
    );
    assert_only_host_delivery_operations(&harness, &[TRIGGER_CREATE]).await;
}

#[tokio::test]
async fn removed_scheduled_target_is_rejected_fail_closed_without_provider_send_fallback() {
    let group = RebornIntegrationGroup::triggers()
        .await
        .expect("trigger group builds");
    let harness = group
        .thread("delivery-scheduled-removed")
        .script([RebornScriptedReply::text("source established")])
        .build()
        .await
        .expect("trigger thread builds");
    let (exact_listed_id, reply_target_binding_ref) = registered_scheduled_target(
        &group,
        &harness,
        "delivery-scheduled-replaceable-provider",
        "opaque-target-removed-after-listing",
    )
    .await;

    group
        .register_source_delivery_target_for_test(
            "delivery-scheduled-replaceable-provider",
            "different-current-target",
            reply_target_binding_ref,
        )
        .expect("replacing the provider removes the formerly listed target");

    harness.push_script([
        RebornScriptedReply::tool_call(
            TRIGGER_CREATE,
            json!({
                "name": "removed target must not persist",
                "prompt": "do not deliver this anywhere else",
                "schedule": {
                    "kind": "once",
                    "at": "2999-01-01T00:00:00",
                    "timezone": "UTC"
                },
                "delivery_target_id": exact_listed_id,
            }),
        ),
        RebornScriptedReply::text("that destination is no longer available"),
    ]);
    harness
        .submit_turn("Schedule to the formerly listed destination.")
        .await
        .expect("recoverable removed-target rejection leaves the turn usable");
    harness
        .assert_tool_error(ToolErrorClass::Failed, "invalid_input")
        .await
        .expect("removed target is rejected by current authority validation");
    assert_only_host_delivery_operations(&harness, &[TRIGGER_CREATE]).await;
}
