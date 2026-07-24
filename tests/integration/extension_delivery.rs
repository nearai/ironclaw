// arch-exempt: large_file, whole-path channel delivery integration journeys, plan #6159
//! Reborn integration test — generic outbound delivery through the REAL
//! coordinator (extension-runtime P5, §5.4 / OUT + DEL-10).
//!
//! Both proofs drive the FULL production inbound→outbound pipeline over the
//! composed runtime: a vendor-signed POST on the production ingress mount →
//! host-side recipe verification → the real channel adapter's normalization →
//! durable admission through the REAL `DefaultProductSurface` → a real turn
//! against a scripted model → the canonical `RunDeliveryEventRouter` and
//! per-channel event handler → the factory-built `DeliveryCoordinator` (sole
//! delivery-state writer, §5.4) →
//! the real adapter's `deliver` → the policy-enforced channel egress with
//! host-side credential injection → the recorded network wire. Assertions
//! land at two seams: the wire recorder (vendor call + injected credential)
//! and the coordinator's outbound-state store (terminal `Delivered` attempt —
//! never `wait_for_status(Completed)` alone).
//!
//! Pinned here, matrixed over libSQL and PostgreSQL (a provisioning failure
//! is a test failure, never a skip):
//! - The Slack proof: a signed DM event yields a `FinalReply` coordinated
//!   through the REAL coordinator to `chat.postMessage`, with the §11
//!   bridged bot token injected host-side (OUT-1/2/5, ING-11 read half).
//!   The Slack lane still owns its ingress registration in production
//!   (setup-store secrets + per-revision sink fed to the assembly as a
//!   lane override), so this test keeps its lane-shaped manual
//!   registration.
//! - The DEL-10 Telegram proof: the bundled telegram package (manifest +
//!   adapter crate only, zero bespoke host code) installs through the
//!   production lifecycle tool, is configured through the PRODUCTION
//!   manifest administrator-configuration port (bot token + webhook secret
//!   into the scoped secret store, webhook URL into the same canonical
//!   configuration projection — zero test-only config injection), activates (`setWebhook`
//!   over recorded egress with host-side path-placeholder substitution of
//!   the configured token) — and the PRODUCTION channel host assembly
//!   (P6 S2) reconciles the activation into an ingress registration
//!   (dynamic administrator-configuration verification secrets + per-extension
//!   durable workflow + run-delivery observer): NO manual sink/observer
//!   registration → a signed update becomes a turn → the reply is
//!   coordinated to `sendMessage` — the "addition test" for a second
//!   production channel. A config re-save while Active proves the §6.5
//!   automatic deactivate → reactivate cycle (a second `setWebhook` with
//!   the new URL).

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use http_body_util::BodyExt;
use ironclaw_host_api::ChannelInboundProductSurface;
use ironclaw_host_api::ProductSurfaceCaller;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, CorrelationId, EffectKind,
    ExecutionContext, ExtensionId, GrantConstraints, InvocationId, InvocationOrigin, MountView,
    NetworkPolicy, Principal, ProductKind, ResourceEstimate, ResourceScope, RuntimeKind,
    TrustClass,
};
use ironclaw_host_runtime::RuntimeCapabilityOutcome;
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_outbound::OutboundDeliveryStatus;
use ironclaw_product::{
    AdapterInstallationId, ChannelAdapter, InboundOutcome, ParsedProductInbound, ProductAdapterId,
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload, ProtocolAuthEvidence,
    UserMessagePayload, VerifiedInbound,
};
use ironclaw_product::{
    ChannelConnectionNoticePolicy, ConversationBindingService, ResolveBindingRequest,
    ResolveStoredProductReplyTargetRequest, RunDeliveryEventHandler, RunDeliveryEventRouter,
    RunDeliveryObserver, RunDeliveryServices, StoredProductReplyTargetAccess,
};
use ironclaw_reborn_composition::{
    ChannelHostAssemblyTestWiring, ChannelHostIdentity, ChannelInboundSinkConfig,
    ChannelIngressRegistration, ExtensionIngressParts, GenericChannelHostAssembly,
    GenericChannelInboundSink, PostAdmissionObserver, RebornRuntime, StaticIngressSecrets,
    VerifiedEvidenceMint, extension_ingress_route_mount,
};
use ironclaw_turns::{GetRunStateRequest, TurnCoordinator, TurnRunId, TurnScope, TurnStatus};
use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use rstest::rstest;
use serde_json::json;
use sha2::Sha256;
use tower::ServiceExt;

const SLACK_ROUTE: &str = "/webhooks/extensions/slack/events";
const SLACK_INSTALLATION: &str = "slack-itest-install";
const SLACK_SIGNING_SECRET: &[u8] = b"itest-slack-signing-secret";
const SLACK_BOT_TOKEN: &str = "xoxb-itest-bot-token";
const SLACK_REPLY: &str = "Here is the coordinated Slack reply.";

const TELEGRAM_ROUTE: &str = "/webhooks/extensions/telegram/updates";
/// The PRODUCTION installation id: the lifecycle facade mints installation
/// ids equal to the extension id, and the assembly's dynamic secrets port
/// reports that id as the verification candidate.
const TELEGRAM_INSTALLATION: &str = "telegram";
const TELEGRAM_WEBHOOK_SECRET: &str = "itest-telegram-webhook-secret";
const TELEGRAM_BOT_TOKEN: &str = "123456:itest-telegram-token";
const TELEGRAM_REPLY: &str = "Here is the coordinated Telegram reply.";

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

async fn wait_for_run_status_in_scope(
    coordinator: &Arc<dyn TurnCoordinator>,
    scope: &TurnScope,
    run_id: TurnRunId,
    expected: TurnStatus,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let state = coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
            .expect("vendor-scoped run state remains readable");
        if state.status == expected {
            return;
        }
        assert!(
            !state.status.is_terminal(),
            "expected {expected:?} but vendor-scoped run reached {:?}; failure={:?}",
            state.status,
            state.failure
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for vendor-scoped run {run_id} to reach {expected:?}; last status={:?}",
            state.status
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Sign a body exactly as the slack manifest's recipe declares: hex
/// HMAC-SHA256 over `v0:{timestamp}:{body}` with a `v0=` prefix.
fn slack_signature(timestamp: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SLACK_SIGNING_SECRET).expect("hmac key");
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body.as_bytes());
    let digest = mac.finalize().into_bytes();
    use std::fmt::Write as _;
    let mut hex = String::new();
    for byte in digest {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    format!("v0={hex}")
}

/// Scripted model for the vendor conversation's run: one static assistant
/// reply, so the observer has a finalized message to deliver.
#[derive(Debug)]
struct StaticReplyGateway(&'static str);

#[async_trait::async_trait]
impl HostManagedModelGateway for StaticReplyGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply(self.0))
    }
}

#[derive(Debug)]
struct PausedReplyGateway {
    reply: &'static str,
    release: tokio::sync::Semaphore,
    run_id: Mutex<Option<TurnRunId>>,
}

impl PausedReplyGateway {
    fn new(reply: &'static str) -> Self {
        Self {
            reply,
            release: tokio::sync::Semaphore::new(0),
            run_id: Mutex::new(None),
        }
    }

    fn release(&self) {
        self.release.add_permits(1);
    }

    async fn wait_for_run_id(&self) -> TurnRunId {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(run_id) = *self.run_id.lock().expect("paused gateway run-id lock") {
                return run_id;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for the paused model request"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[async_trait::async_trait]
impl HostManagedModelGateway for PausedReplyGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        *self.run_id.lock().expect("paused gateway run-id lock") = Some(request.run_id);
        let permit = self
            .release
            .acquire()
            .await
            .expect("paused test gateway semaphore remains open");
        permit.forget();
        Ok(HostManagedModelResponse::assistant_reply(self.reply))
    }
}

/// Post-admission observer that records every ack, forwards admission-time
/// feedback to the REAL generic observer, and reconciles accepted runs onto
/// the canonical lifecycle router. This mirrors the production channel host's
/// `RunDeliveryPostAdmissionObserver` while retaining admission assertions.
struct RecordingForwardObserver {
    acks: Mutex<Vec<ProductInboundAck>>,
    errors: Mutex<Vec<String>>,
    inner: Arc<RunDeliveryObserver>,
    event_handler: Arc<RunDeliveryEventHandler>,
    event_router: Arc<RunDeliveryEventRouter>,
}

impl RecordingForwardObserver {
    fn new(
        inner: Arc<RunDeliveryObserver>,
        event_handler: Arc<RunDeliveryEventHandler>,
        event_router: Arc<RunDeliveryEventRouter>,
    ) -> Self {
        Self {
            acks: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            inner,
            event_handler,
            event_router,
        }
    }

    fn accepted_count(&self) -> usize {
        self.acks
            .lock()
            .expect("acks lock")
            .iter()
            .filter(|ack| matches!(ack, ProductInboundAck::Accepted { .. }))
            .count()
    }

    fn errors(&self) -> Vec<String> {
        self.errors.lock().expect("errors lock").clone()
    }

    fn accepted_run_id(&self) -> Option<ironclaw_turns::TurnRunId> {
        self.acks
            .lock()
            .expect("acks lock")
            .iter()
            .find_map(|ack| match ack {
                ProductInboundAck::Accepted {
                    submitted_run_id, ..
                } => Some(*submitted_run_id),
                _ => None,
            })
    }
}

#[async_trait::async_trait]
impl PostAdmissionObserver for RecordingForwardObserver {
    async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        self.acks.lock().expect("acks lock").push(ack.clone());
        self.inner.observe_ack(envelope.clone(), ack.clone()).await;
        self.event_handler
            .reconcile_accepted_user_message(self.event_router.as_ref(), &envelope, &ack)
            .await
            .expect("accepted external turn reconciles onto the lifecycle router");
    }

    async fn observe_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ironclaw_product::ProductAdapterError,
    ) {
        self.errors
            .lock()
            .expect("errors lock")
            .push(format!("{error:?}"));
        self.inner.observe_error(envelope, error).await;
    }
}

/// Generic run-delivery services over the REAL runtime pieces: the group's
/// binding/thread/turn services (the world the admitted run executes in)
/// plus the composed runtime's coordinator and outbound stores (the SAME
/// instances the factory wired — observer and coordinator share one
/// delivery ledger).
fn delivery_run_services(
    harness: &RebornIntegrationHarness,
    services: &RebornRuntime,
    extension_id: &str,
) -> RunDeliveryServices {
    let (outbound_store, route_store, communication_preferences) = services
        .outbound_delivery_stores_for_test()
        .expect("composed runtime exposes the coordinator's outbound stores");
    let coordinator = services
        .delivery_coordinator()
        .expect("composition built the delivery coordinator");
    let fallback_notice_scope = TurnScope::new_with_owner(
        harness.binding.tenant_id.clone(),
        harness.binding.agent_id.clone(),
        harness.binding.project_id.clone(),
        ironclaw_host_api::ThreadId::new(format!("{extension_id}-itest-channel-notices"))
            .expect("notice thread id"),
        harness.binding.subject_user_id.clone(),
    );
    RunDeliveryServices {
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
        extension_id: extension_id.to_string(),
        fallback_notice_scope,
        approval_context: None,
        blocked_auth_prompts: None,
        auth_flow_cancel: None,
    }
}

/// Predict the vendor conversation's turn scope BEFORE posting: normalize
/// the exact wire body through the REAL adapter, assemble the envelope
/// exactly as `GenericChannelInboundSink::admit` does, and resolve the same
/// durable binding the workflow will find at admission (through the SAME
/// binding service the registered sink uses) — so the scripted model
/// gateway can be registered for the run's scope up front.
async fn preresolve_vendor_turn_scope(
    binding_service: &Arc<dyn ConversationBindingService>,
    adapter: &dyn ChannelAdapter,
    adapter_id: &str,
    installation_id: &str,
    evidence: &ProtocolAuthEvidence,
    body: &str,
) -> TurnScope {
    let outcome = adapter
        .inbound(VerifiedInbound {
            extension_id: adapter_id,
            installation_id,
            body: body.as_bytes(),
            headers: &[],
        })
        .expect("the vendor body must parse through the real adapter");
    let InboundOutcome::Messages(messages) = outcome else {
        panic!("the vendor body must normalize to messages");
    };
    let message = messages.first().expect("one normalized message");
    // Mirror of the sink's envelope assembly (`extension_ingress.rs::admit`).
    let context = ironclaw_product::TrustedInboundContext::from_verified_evidence(
        ProductAdapterId::new(adapter_id).expect("adapter id"),
        AdapterInstallationId::new(installation_id).expect("installation id"),
        Utc::now(),
        evidence,
    )
    .expect("trusted inbound context");
    let payload = ProductInboundPayload::UserMessage(
        UserMessagePayload::new(message.text.clone(), Vec::new(), message.trigger)
            .expect("user message payload"),
    );
    let parsed = ParsedProductInbound::new(
        message.event_id.clone(),
        message.actor.clone(),
        message.conversation.clone(),
        payload,
    )
    .expect("parsed inbound");
    let envelope =
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("inbound envelope");
    let binding = binding_service
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect("vendor conversation binding resolves");
    TurnScope::new_with_owner(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        binding.thread_id.clone(),
        binding.subject_user_id.clone(),
    )
}

struct VendorIngress {
    parts: ExtensionIngressParts,
    mount: ironclaw_host_ingress::PublicRouteMount,
}

impl VendorIngress {
    /// Register one extension's inbound wiring — static verification secret
    /// plus the generic sink over THIS harness's real workflow, observed by
    /// the REAL run-delivery observer — and build the production route mount.
    fn register(
        parts: ExtensionIngressParts,
        extension_id: &str,
        installation_id: &str,
        secret: &[u8],
        evidence: VerifiedEvidenceMint,
        harness: &RebornIntegrationHarness,
        observer: Arc<RecordingForwardObserver>,
    ) -> Self {
        let surface = harness.product_workflow_for_test() as Arc<dyn ChannelInboundProductSurface>;
        let sink = Arc::new(GenericChannelInboundSink::new(ChannelInboundSinkConfig {
            adapter_id: ProductAdapterId::new(extension_id).expect("adapter id"),
            evidence,
            surface,
            observer: Some(observer as Arc<dyn PostAdmissionObserver>),
        }));
        parts.registry.register(
            extension_id,
            ChannelIngressRegistration {
                secrets: Arc::new(StaticIngressSecrets::new(vec![
                    ironclaw_extension_host::ingress::VerificationCandidate {
                        installation_id: installation_id.to_string(),
                        secret: secret.to_vec(),
                    },
                ])),
                sink: sink.clone() as Arc<dyn ironclaw_extension_host::ingress::InboundSink>,
                drain: Some(sink as Arc<dyn ironclaw_reborn_composition::ChannelIngressDrain>),
            },
        );
        let mount = extension_ingress_route_mount(&parts).expect("production mount builds");
        Self { parts, mount }
    }

    /// The production mount over the composed ingress WITHOUT any manual
    /// registration — the S2 shape: the production channel host assembly
    /// owns the per-extension registrations.
    fn production(parts: ExtensionIngressParts) -> Self {
        let mount = extension_ingress_route_mount(&parts).expect("production mount builds");
        Self { parts, mount }
    }

    async fn post(
        &self,
        route: &str,
        body: &str,
        headers: Vec<(&'static str, String)>,
    ) -> StatusCode {
        self.post_with_body(route, body, headers).await.0
    }

    async fn post_with_body(
        &self,
        route: &str,
        body: &str,
        headers: Vec<(&'static str, String)>,
    ) -> (StatusCode, String) {
        let mut builder = Request::builder().method("POST").uri(route);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        let response = self
            .mount
            .router
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).expect("request"))
            .await
            .expect("router responds");
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body collects")
            .to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    /// Await every spawned post-admission observer — the full outbound
    /// delivery runs inside those tasks, so after this the wire and the
    /// outbound store are settled.
    async fn drain(&self) {
        self.parts.registry.drain().await;
    }
}

/// Install the REAL bundled Slack package through the production lifecycle
/// tool. Install completes readiness/publication internally, so the
/// coordinator's snapshot resolver sees an active channel binding.
async fn activate_slack(group: &RebornIntegrationGroup) {
    let lifecycle = group
        .thread("conv-slack-delivery-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("installed and ready"),
        ])
        .build()
        .await
        .expect("slack lifecycle thread builds");
    lifecycle
        .seed_capability_credential_account(
            "slack",
            "slack delivery account",
            &[
                "search:read",
                "channels:history",
                "groups:history",
                "im:history",
                "mpim:history",
                "channels:read",
                "groups:read",
                "im:read",
                "mpim:read",
                "users:read",
                "chat:write",
            ],
        )
        .await
        .expect("seed slack account");
    lifecycle
        .submit_turn("install slack")
        .await
        .expect("slack install completes");
    lifecycle
        .assert_tool_result_contains("\"installed\":true")
        .await
        .expect("slack install reported success");
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await
        .expect("slack install completed readiness and publication");
}

/// Assert the coordinator's ledger for `scope`: at least one attempt reached
/// terminal `Delivered`, and none is stranded mid-lifecycle
/// (`Prepared`/`Sending` — persist-before-egress must settle terminally).
async fn assert_delivered_attempt(services: &RebornRuntime, scope: &TurnScope) {
    let (outbound_store, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("outbound stores");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let attempts = loop {
        let attempts = outbound_store
            .list_delivery_attempts(scope.clone())
            .await
            .expect("list delivery attempts");
        let has_delivered = attempts
            .iter()
            .any(|attempt| attempt.status == OutboundDeliveryStatus::Delivered);
        let all_terminal = attempts.iter().all(|attempt| {
            !matches!(
                attempt.status,
                OutboundDeliveryStatus::Prepared
                    | OutboundDeliveryStatus::Sending
                    | OutboundDeliveryStatus::Pending
            )
        });
        if has_delivered && all_terminal {
            break attempts;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for a terminal Delivered attempt; got {:?}",
            attempts
                .iter()
                .map(|attempt| attempt.status)
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt.status == OutboundDeliveryStatus::Delivered),
        "the coordinator must record a terminal Delivered attempt; got {:?}",
        attempts
            .iter()
            .map(|attempt| attempt.status)
            .collect::<Vec<_>>()
    );
    assert!(
        attempts.iter().all(|attempt| !matches!(
            attempt.status,
            OutboundDeliveryStatus::Prepared
                | OutboundDeliveryStatus::Sending
                | OutboundDeliveryStatus::Pending
        )),
        "no attempt may be stranded mid-lifecycle after drain; got {:?}",
        attempts
            .iter()
            .map(|attempt| attempt.status)
            .collect::<Vec<_>>()
    );
}

/// Await the production assembly's reconcile: deployment discovery or an
/// active-snapshot change registers the extension's inbound wiring, and the
/// per-extension binding service becomes readable. Bounded — a missing
/// registration is a test failure, never a hang.
async fn wait_for_production_registration(
    assembly: &Arc<GenericChannelHostAssembly>,
    services: &RebornRuntime,
    extension_id: &str,
) -> Arc<dyn ConversationBindingService> {
    let registry = services
        .extension_ingress_parts()
        .expect("composition built the generic ingress")
        .registry;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if registry.is_registered(extension_id)
            && let Some(binding) = assembly.binding_service_for_extension_for_test(extension_id)
        {
            return binding;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the production assembly must register `{extension_id}`'s ingress"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn reborn_services(group: &RebornIntegrationGroup) -> &RebornRuntime {
    group
        .capability_harness()
        .expect("host-runtime capability harness")
        .reborn_services_for_test()
        .expect("composed reborn services")
}

async fn configure_admin_group(
    group: &RebornIntegrationGroup,
    group_id: &str,
    expected_revision: u64,
    values: serde_json::Value,
) {
    let services = reborn_services(group);
    // `extension_delivery()` composes its local runtime with this service
    // label as the tenant operator. Its ordinary capability executor uses a
    // distinct user to prove caller scoping, so admin ingress must deliberately
    // use the composition owner rather than that executor identity.
    let operator_user_id = ironclaw_host_api::UserId::new("reborn-e2e-extension-lifecycle-tools")
        .expect("delivery profile operator user id");
    let capability_id = CapabilityId::new("builtin.admin_configuration_replace")
        .expect("admin configuration capability id");
    let product_ingress = ExtensionId::new("ironclaw_webui").expect("product ingress id");
    let invocation_id = InvocationId::new();
    let runtime_scope = &group.shared.product_harness.scope;
    let runtime_agent_id = runtime_scope
        .agent_id
        .clone()
        .expect("delivery profile runtime scope has an agent id");
    let scope = ResourceScope {
        // Admin configuration is deployment/tenant shared. The delivery group
        // aligns the composed runtime's tenant/agent with the product harness
        // scope, so write through that runtime identity rather than a
        // hardcoded local-dev default.
        tenant_id: runtime_scope.tenant_id.clone(),
        user_id: operator_user_id.clone(),
        agent_id: Some(runtime_agent_id),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    let context = ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: operator_user_id.clone(),
        authenticated_actor_user_id: Some(operator_user_id),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: None,
        thread_id: None,
        run_id: None,
        origin: Some(InvocationOrigin::Product(
            ProductKind::new("webui").expect("product origin kind"),
        )),
        extension_id: product_ingress.clone(),
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::Sandbox,
        grants: CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: capability_id.clone(),
                grantee: Principal::Extension(product_ingress),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![
                        EffectKind::ReadFilesystem,
                        EffectKind::WriteFilesystem,
                        EffectKind::DeleteFilesystem,
                        EffectKind::UseSecret,
                    ],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: Some(1),
                },
            }],
        },
        mounts: MountView::default(),
        resource_scope: scope,
    };
    context
        .validate()
        .expect("admin capability context validates");
    let outcome = services
        .host_runtime_for_test()
        .expect("host runtime")
        .invoke_capability((
            context,
            capability_id,
            ResourceEstimate::default(),
            json!({
                "group_id": group_id,
                "expected_revision": expected_revision,
                "values": values,
            }),
        ))
        .await
        .expect("admin configuration dispatch completes");
    assert!(
        matches!(outcome, RuntimeCapabilityOutcome::Completed(_)),
        "admin configuration must complete through the authorized runtime, got {outcome:?}"
    );
}

async fn assert_extension_has_no_user_installation(services: &RebornRuntime, extension_id: &str) {
    let installations = services
        .extension_installation_store_for_test()
        .expect("local extension installation store")
        .list_installations()
        .await
        .expect("list extension installations");
    assert!(
        installations
            .iter()
            .all(|installation| installation.extension_id().as_str() != extension_id),
        "admin configuration must not create or activate a user installation for {extension_id}"
    );
}

fn start_channel_host_assembly(
    group: &RebornIntegrationGroup,
    services: &RebornRuntime,
    inbound: &RebornIntegrationHarness,
) -> Arc<GenericChannelHostAssembly> {
    services
        .start_channel_host_assembly_for_test(ChannelHostAssemblyTestWiring {
            thread_service: inbound
                .thread_service_for_test()
                .expect("group thread service"),
            turn_coordinator: inbound.turn_coordinator_for_test(),
            run_delivery_events: group
                .run_delivery_events()
                .expect("delivery group wires the canonical run-delivery event router"),
            identity: ChannelHostIdentity {
                tenant_id: inbound.binding.tenant_id.clone(),
                agent_id: inbound.binding.agent_id.clone().expect("binding agent id"),
                project_id: inbound.binding.project_id.clone(),
                operator_user_id: inbound
                    .binding
                    .subject_user_id
                    .clone()
                    .expect("binding subject user id"),
            },
        })
        .expect("production channel host assembly starts")
}

#[tokio::test]
async fn admin_configured_slack_unconnected_dm_gets_connect_notice_without_installation_or_turn() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let services = reborn_services(&group);
    let inbound = group
        .thread("conv-admin-slack-unconnected")
        .script([RebornScriptedReply::text("must stay unused")])
        .build()
        .await
        .expect("inbound thread builds");
    assert_extension_has_no_user_installation(services, "slack").await;
    let assembly = start_channel_host_assembly(&group, services, &inbound);
    let _binding = wait_for_production_registration(&assembly, services, "slack").await;
    let ingress = VendorIngress::production(
        services
            .extension_ingress_parts()
            .expect("composition built generic ingress"),
    );

    let unconfigured_body = "{}";
    let unconfigured_timestamp = now_unix().to_string();
    let unconfigured_signature = slack_signature(&unconfigured_timestamp, unconfigured_body);
    let (unconfigured_status, unconfigured_response) = ingress
        .post_with_body(
            SLACK_ROUTE,
            unconfigured_body,
            vec![
                ("X-Slack-Signature", unconfigured_signature),
                ("X-Slack-Request-Timestamp", unconfigured_timestamp),
            ],
        )
        .await;
    assert_eq!(
        unconfigured_status,
        StatusCode::UNAUTHORIZED,
        "the manifest route must exist but fail closed before admin configuration: {unconfigured_response}"
    );

    configure_admin_group(
        &group,
        "extension.slack",
        0,
        json!([
            {"handle": "slack_bot_token", "value": SLACK_BOT_TOKEN},
            {"handle": "slack_signing_secret", "value": String::from_utf8_lossy(SLACK_SIGNING_SECRET)},
            {"handle": "slack_team_id", "value": "T-A"},
            {"handle": "slack_api_app_id", "value": "A-ITEST"},
            {"handle": "slack_installation_id", "value": SLACK_INSTALLATION},
            {"handle": "slack_bot_user_id", "value": "U-BOT"},
            {"handle": "slack_oauth_client_id", "value": "slack-oauth-client"},
            {"handle": "slack_oauth_client_secret", "value": "slack-oauth-secret"}
        ]),
    )
    .await;
    assert_extension_has_no_user_installation(services, "slack").await;
    let message = "admin-configured Slack DM must not reach the agent";
    let body = json!({
        "type": "event_callback",
        "event_id": "Ev-admin-slack-unconnected",
        "team_id": "T-A",
        "event": {
            "type": "message",
            "user": "U-UNCONNECTED",
            "channel": "D-UNCONNECTED",
            "channel_type": "im",
            "text": message,
            "ts": "1710000500.000100"
        }
    })
    .to_string();
    let timestamp = now_unix().to_string();
    let signature = slack_signature(&timestamp, &body);
    let (status, response_body) = ingress
        .post_with_body(
            SLACK_ROUTE,
            &body,
            vec![
                ("X-Slack-Signature", signature),
                ("X-Slack-Request-Timestamp", timestamp),
            ],
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "admin-configured Slack route response: {response_body}"
    );
    ingress.drain().await;
    let notice = ChannelConnectionNoticePolicy::generic("Slack");
    assert!(
        inbound
            .captured_network_requests_for_test()
            .iter()
            .any(|request| {
                request.url.ends_with("/api/chat.postMessage")
                    && String::from_utf8_lossy(&request.body)
                        .contains(notice.connect_required.as_str())
            }),
        "the unconnected Slack DM must receive the manifest/generic connect notice"
    );
    assert!(
        inbound
            .assert_model_request_contains(message)
            .await
            .is_err(),
        "the unconnected Slack DM must not admit an agent turn"
    );
    assert_extension_has_no_user_installation(services, "slack").await;
}

#[tokio::test]
async fn admin_configured_telegram_unconnected_dm_gets_connect_notice_without_installation_or_turn()
{
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let services = reborn_services(&group);
    let inbound = group
        .thread("conv-admin-telegram-unconnected")
        .script([RebornScriptedReply::text("must stay unused")])
        .build()
        .await
        .expect("inbound thread builds");
    assert_extension_has_no_user_installation(services, "telegram").await;
    let assembly = start_channel_host_assembly(&group, services, &inbound);
    let _binding = wait_for_production_registration(&assembly, services, "telegram").await;
    let ingress = VendorIngress::production(
        services
            .extension_ingress_parts()
            .expect("composition built generic ingress"),
    );

    let (unconfigured_status, unconfigured_response) = ingress
        .post_with_body(
            TELEGRAM_ROUTE,
            "{}",
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(
        unconfigured_status,
        StatusCode::UNAUTHORIZED,
        "the manifest route must exist but fail closed before admin configuration: {unconfigured_response}"
    );

    configure_admin_group(
        &group,
        "extension.telegram",
        0,
        json!([
            {"handle": "telegram_bot_token", "value": TELEGRAM_BOT_TOKEN},
            {"handle": "telegram_webhook_secret", "value": TELEGRAM_WEBHOOK_SECRET},
            {"handle": "telegram_webhook_url", "value": "https://hooks.example.test/webhooks/extensions/telegram/updates"},
            {"handle": "bot_username", "value": "itest_admin_bot"}
        ]),
    )
    .await;
    assert_extension_has_no_user_installation(services, "telegram").await;
    let message = "admin-configured Telegram DM must not reach the agent";
    let body = json!({
        "update_id": 7001,
        "message": {
            "message_id": 7011,
            "date": 1710000000,
            "text": message,
            "from": {"id": 700700, "is_bot": false, "first_name": "Pat"},
            "chat": {"id": 700700, "type": "private"}
        }
    })
    .to_string();
    let (status, response_body) = ingress
        .post_with_body(
            TELEGRAM_ROUTE,
            &body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "admin-configured Telegram route response: {response_body}"
    );
    ingress.drain().await;
    let notice = services
        .pairing_connection_notices_for_test("telegram")
        .expect("the bundled manifest composes Telegram's pairing notices");
    assert!(
        inbound
            .captured_network_requests_for_test()
            .iter()
            .any(|request| {
                request.url.ends_with("/sendMessage")
                    && String::from_utf8_lossy(&request.body)
                        .contains(notice.connect_required.as_str())
            }),
        "the unconnected Telegram DM must receive the manifest/generic connect notice"
    );
    assert!(
        inbound
            .assert_model_request_contains(message)
            .await
            .is_err(),
        "the unconnected Telegram DM must not admit an agent turn"
    );
    assert_extension_has_no_user_installation(services, "telegram").await;
}

/// The Slack outbound proof (OUT-1/2/5 + ING-11 read half): a signed DM
/// event on the production mount becomes a real turn whose `FinalReply` is
/// coordinated through the REAL factory-built `DeliveryCoordinator` to
/// `chat.postMessage`, with the §11 bridged bot token injected host-side —
/// asserted on the wire recorder AND in the coordinator's outbound store.
#[rstest]
#[case::libsql(StorageMode::LibSql)]
#[case::postgres(StorageMode::Postgres)]
#[tokio::test(flavor = "multi_thread")]
async fn slack_final_reply_flows_through_the_real_delivery_coordinator(
    #[case] storage: StorageMode,
) {
    let group = RebornIntegrationGroup::builder()
        .storage(storage)
        .extension_delivery()
        .await
        .expect("delivery group builds on this backend");
    activate_slack(&group).await;
    let services = reborn_services(&group);
    assert!(
        services.register_static_channel_egress_credentials_for_test(vec![(
            "slack".to_string(),
            "slack_bot_token".to_string(),
            ironclaw_secrets::SecretMaterial::from(SLACK_BOT_TOKEN.to_string()),
        )]),
        "the composed runtime must expose channel-egress credential bridging"
    );

    let inbound = group
        .thread("conv-slack-delivery-inbound")
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await
        .expect("inbound thread builds");
    let event_router = group
        .run_delivery_events()
        .expect("delivery group wires the canonical run-delivery event router");
    let delivery_services = delivery_run_services(&inbound, services, "slack");
    let event_handler = Arc::new(RunDeliveryEventHandler::new(
        delivery_services.clone(),
        "slack",
        SLACK_INSTALLATION,
    ));
    event_router.register("slack", &event_handler);
    let observer = Arc::new(RecordingForwardObserver::new(
        Arc::new(RunDeliveryObserver::new(delivery_services)),
        event_handler,
        Arc::clone(&event_router),
    ));
    let ingress = VendorIngress::register(
        services
            .extension_ingress_parts()
            .expect("composition built the generic ingress"),
        "slack",
        SLACK_INSTALLATION,
        SLACK_SIGNING_SECRET,
        VerifiedEvidenceMint::RequestSignature {
            signature_header: "X-Slack-Signature".to_string(),
            timestamp_header: Some("X-Slack-Request-Timestamp".to_string()),
        },
        &inbound,
        Arc::clone(&observer),
    );

    let body = json!({
        "type": "event_callback",
        "event_id": "Ev-delivery-slack-1",
        "team_id": "T-A",
        "event": {
            "type": "message",
            "user": "U777",
            "channel": "D777",
            "channel_type": "im",
            "text": "please reply through the coordinator",
            "ts": "1710000300.000100"
        }
    })
    .to_string();
    // The run's scope is the vendor conversation's binding, not this harness
    // thread's — register its scripted model before the POST admits the turn.
    let evidence = ironclaw_product::auth::mark_request_signature_verified(
        "X-Slack-Signature".to_string(),
        Some("X-Slack-Request-Timestamp".to_string()),
        SLACK_INSTALLATION,
    );
    let slack_binding_service = inbound
        .binding_service_for_test()
        .expect("group binding service");
    let vendor_scope = preresolve_vendor_turn_scope(
        &slack_binding_service,
        &ironclaw_slack_extension::SlackChannelAdapter,
        "slack",
        SLACK_INSTALLATION,
        &evidence,
        &body,
    )
    .await;
    inbound.register_scope_gateway_for_test(
        vendor_scope.clone(),
        Arc::new(StaticReplyGateway(SLACK_REPLY)),
    );

    let timestamp = now_unix().to_string();
    let signature = slack_signature(&timestamp, &body);
    let status = ingress
        .post(
            SLACK_ROUTE,
            &body,
            vec![
                ("X-Slack-Signature", signature),
                ("X-Slack-Request-Timestamp", timestamp),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "the signed event must be accepted");
    ingress.drain().await;
    assert_eq!(
        observer.accepted_count(),
        1,
        "the signed DM must be admitted as a turn (errors: {:?})",
        observer.errors()
    );
    let run_id = observer
        .accepted_run_id()
        .expect("the accepted Slack event must identify its submitted run");
    let coordinator = inbound.turn_coordinator_for_test();
    wait_for_run_status_in_scope(&coordinator, &vendor_scope, run_id, TurnStatus::Completed).await;
    let completed = coordinator
        .get_run_state(GetRunStateRequest {
            scope: vendor_scope.clone(),
            run_id,
        })
        .await
        .expect("completed Slack run remains readable");
    let actor = completed.actor.clone().expect("completed Slack run actor");
    assert_eq!(
        Some(&actor.user_id),
        vendor_scope.explicit_owner_user_id(),
        "the admitted run actor must remain the exact user paired to the source route"
    );
    let resolved_target = slack_binding_service
        .resolve_stored_reply_target(ResolveStoredProductReplyTargetRequest {
            scope: vendor_scope.clone(),
            actor,
            reply_target_binding_ref: completed.reply_target_binding_ref.clone(),
            access: StoredProductReplyTargetAccess::OrdinaryReply,
        })
        .await
        .expect("the admitted Slack source route remains authorized");
    assert_eq!(resolved_target.adapter_id.as_str(), "slack");
    assert_eq!(resolved_target.installation_id.as_str(), SLACK_INSTALLATION);
    assert_delivered_attempt(services, &vendor_scope).await;
    event_router.wait_until_run_idle(run_id).await;

    // Wire seam: the coordinated FinalReply reached chat.postMessage with the
    // bridged bot token injected host-side (the adapter never saw it).
    // #6520 delivery is event-driven, so poll the wire with the file's
    // bounded deadline instead of a single post-idle snapshot.
    let wire_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let (requests, post_message_position) = loop {
        let requests = inbound.captured_network_requests_for_test();
        if let Some(position) = requests.iter().position(|request| {
            request.url.ends_with("/api/chat.postMessage")
                && String::from_utf8_lossy(&request.body).contains(SLACK_REPLY)
        }) {
            break (requests, position);
        }
        assert!(
            tokio::time::Instant::now() < wire_deadline,
            "chat.postMessage with the reply must land on the wire; got {:?}",
            requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let post_message = &requests[post_message_position];
    let posted_body = String::from_utf8_lossy(&post_message.body);
    assert!(
        posted_body.contains("\"channel\":\"D777\""),
        "the reply must target the originating DM conversation; got {posted_body}"
    );
    let authorization = post_message
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .expect("host-side credential injection must add the authorization header");
    assert_eq!(authorization.1, format!("Bearer {SLACK_BOT_TOKEN}"));
}

/// DEL-10: the bundled Telegram package — one manifest plus the adapter
/// crate, zero bespoke host code — installs through the production
/// lifecycle tool, consumes the authorized manifest-driven administrator
/// configuration, activates (`setWebhook` over recorded egress with the
/// CONFIGURED bot token substituted host-side into the URL path
/// placeholder), receives a signed update through the production router
/// mount, runs a real turn, delivers the reply through the generic
/// lifecycle router → REAL coordinator → `sendMessage`, and refreshes the active
/// adapter after a later authorized administrator update.
#[rstest]
#[case::libsql(StorageMode::LibSql)]
#[case::postgres(StorageMode::Postgres)]
#[tokio::test(flavor = "multi_thread")]
async fn telegram_update_becomes_a_turn_and_a_coordinated_reply(#[case] storage: StorageMode) {
    Box::pin(telegram_update_becomes_a_turn_and_a_coordinated_reply_impl(
        storage,
    ))
    .await;
}

async fn telegram_update_becomes_a_turn_and_a_coordinated_reply_impl(storage: StorageMode) {
    let group = RebornIntegrationGroup::builder()
        .storage(storage)
        .extension_delivery()
        .await
        .expect("delivery group builds on this backend");
    let services = reborn_services(&group);

    // The inbound thread first: its wire baseline precedes activation, so
    // `captured_network_requests_for_test` sees the setWebhook call too.
    let inbound = group
        .thread("conv-telegram-delivery-inbound")
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await
        .expect("inbound thread builds");
    let event_router = group
        .run_delivery_events()
        .expect("delivery group wires the canonical run-delivery event router");

    // Attach the PRODUCTION channel host assembly (P6 S2) over the composed
    // runtime. The harness supplies only its run-world services — the
    // group's shared turn runtime executes the admitted runs — while the
    // snapshot watch, ingress registry, administrator-configuration secret storage,
    // durable workflow substrate, and delivery coordinator + outbound
    // stores are the production wiring. From here NOTHING registers the
    // telegram sink or observer manually.
    let assembly = services
        .start_channel_host_assembly_for_test(ChannelHostAssemblyTestWiring {
            thread_service: inbound
                .thread_service_for_test()
                .expect("group thread service"),
            turn_coordinator: inbound.turn_coordinator_for_test(),
            run_delivery_events: Arc::clone(&event_router),
            identity: ChannelHostIdentity {
                tenant_id: inbound.binding.tenant_id.clone(),
                agent_id: inbound.binding.agent_id.clone().expect("binding agent id"),
                project_id: inbound.binding.project_id.clone(),
                operator_user_id: inbound
                    .binding
                    .subject_user_id
                    .clone()
                    .expect("binding subject user id"),
            },
        })
        .expect("the production channel host assembly starts over the composed runtime");

    // Admin configuration is a separate tenant axis and is valid before any
    // user installs the channel. The user then installs once; that one action
    // parks on personal pairing and resumes to active after pairing.
    let lifecycle = group
        .thread("conv-telegram-delivery-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "telegram"}),
            ),
            RebornScriptedReply::text("installed and ready"),
        ])
        .build()
        .await
        .expect("telegram lifecycle thread builds");

    // Deployment-owned values cross the authorized administrator capability,
    // never the caller's personal setup surface.
    configure_admin_group(
        &group,
        "extension.telegram",
        0,
        json!([
            {"handle": "telegram_webhook_url", "value": "https://hooks.example.test/webhooks/extensions/telegram/updates"},
            {"handle": "telegram_bot_token", "value": TELEGRAM_BOT_TOKEN},
            {"handle": "telegram_webhook_secret", "value": TELEGRAM_WEBHOOK_SECRET},
            {"handle": "bot_username", "value": "itest_delivery_bot"}
        ]),
    )
    .await;

    let (activation_run_id, _activation_gate_ref) = lifecycle
        .submit_turn_until_auth_blocked("install telegram")
        .await
        .expect("unpaired Telegram install parks on its pairing requirement");
    let activation_state = lifecycle
        .wait_for_status(activation_run_id, ironclaw_turns::TurnStatus::BlockedAuth)
        .await
        .expect("Telegram install remains blocked while the caller is unpaired");
    assert!(
        activation_state
            .credential_requirements
            .iter()
            .any(|requirement| matches!(
                (&requirement.setup, requirement.provider.as_str()),
                (
                    ironclaw_host_api::RuntimeCredentialAccountSetup::Pairing,
                    "telegram"
                )
            )),
        "Telegram activation gate must preserve the manifest-declared pairing setup and provider: {:?}",
        activation_state.credential_requirements
    );
    let paired_user = inbound
        .binding
        .subject_user_id
        .clone()
        .expect("binding subject user id");
    assert_eq!(
        activation_state.scope.explicit_owner_user_id(),
        Some(&paired_user),
        "pairing completion and lifecycle activation must share the explicit owner scope"
    );
    let installation_store = services
        .extension_installation_store_for_test()
        .expect("extension delivery profile carries the lifecycle store");
    let installation_id = ironclaw_extensions::ExtensionInstallationId::new(TELEGRAM_INSTALLATION)
        .expect("Telegram installation id");
    let installation = installation_store
        .get_installation(&installation_id)
        .await
        .expect("Telegram installation state reads")
        .expect("Telegram remains installed while activation is blocked");
    assert!(installation.owner().visible_to(&paired_user));
    assert!(
        inbound
            .captured_network_requests_for_test()
            .iter()
            .all(|request| !request.url.ends_with("/setWebhook")),
        "the publication hook must not run before pairing"
    );

    // The pairing surface remains usable while activation is parked. These
    // are the exact product-safe inputs the WebGeneratedCode UI turns into
    // the code, deep-link/QR, and expiry countdown.
    let (pairing_code, pairing_deep_link, pairing_expires_at) = services
        .pairing_issue_for_test("telegram", &paired_user)
        .await
        .expect("installed Telegram exposes its WebGeneratedCode pairing issue");
    let expected_deep_link = format!("https://t.me/itest_delivery_bot?start={pairing_code}");
    assert_eq!(
        pairing_deep_link.as_deref(),
        Some(expected_deep_link.as_str()),
        "the manifest template and configured username must survive install gating"
    );
    let now = Utc::now();
    assert!(
        pairing_expires_at > now && pairing_expires_at <= now + chrono::Duration::minutes(16),
        "pairing expiry must remain a live countdown input: {pairing_expires_at}"
    );

    let paired = services
        .pairing_consume_for_test(
            "telegram",
            TELEGRAM_INSTALLATION,
            &pairing_code,
            ("user", "9911", None, "8675309"),
            (
                lifecycle.turn_coordinator_for_test(),
                lifecycle.turn_state_store_for_test(),
                inbound.binding.tenant_id.clone(),
            ),
        )
        .await
        .expect("pairing consume dispatches its continuation");
    assert_eq!(paired.as_ref(), Some(&paired_user));
    lifecycle
        .wait_for_status(activation_run_id, ironclaw_turns::TurnStatus::Completed)
        .await
        .expect("pairing continuation resumes the exact blocked install");
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await
        .expect("telegram install completed readiness and publication");

    // Activation seam: setWebhook crossed the recorded wire with the bot
    // token substituted host-side into the URL path (the adapter only ever
    // names the `{telegram_bot_token}` placeholder).
    let requests = inbound.captured_network_requests_for_test();
    let set_webhook = requests
        .iter()
        .find(|request| request.url.ends_with("/setWebhook"))
        .unwrap_or_else(|| {
            panic!(
                "activation must call setWebhook over recorded egress; got {:?}",
                requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
            )
        });
    assert_eq!(
        set_webhook.url,
        format!("https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/setWebhook"),
        "the path placeholder must be substituted host-side"
    );
    assert!(
        String::from_utf8_lossy(&set_webhook.body)
            .contains("https://hooks.example.test/webhooks/extensions/telegram/updates"),
        "setWebhook must register the configured public webhook URL"
    );
    // The Telegram contract takes `secret_token` (the VALUE Telegram echoes
    // back on every webhook delivery); the adapter only ever names the
    // handle, and the host resolves it into the JSON body through the
    // manifest-declared body-credential binding. Without the real value the
    // webhook registers secretless and the shared_secret_header verifier
    // rejects every genuine update.
    let set_webhook_body = String::from_utf8_lossy(&set_webhook.body);
    assert!(
        set_webhook_body.contains(&format!("\"secret_token\":\"{TELEGRAM_WEBHOOK_SECRET}\"")),
        "setWebhook must carry the configured webhook secret value, resolved host-side; got {set_webhook_body}"
    );
    assert!(
        !set_webhook_body.contains("secret_token_handle"),
        "the credential handle name must never reach the vendor; got {set_webhook_body}"
    );
    // Redaction: the wire carries the secret by contract, but the
    // model-visible install result must not.
    let activation_output = lifecycle
        .tool_result_output("builtin.extension_install")
        .await
        .expect("install tool output");
    assert!(
        !activation_output
            .to_string()
            .contains(TELEGRAM_WEBHOOK_SECRET),
        "the webhook secret must not appear in model-visible tool output; got {activation_output}"
    );

    // The PRODUCTION assembly reconciled the activation into an ingress
    // registration: dynamic administrator-configuration verification secrets, the
    // per-extension durable workflow, and the run-delivery observer — this
    // test registers nothing.
    let telegram_binding_service =
        wait_for_production_registration(&assembly, services, "telegram").await;
    let ingress = VendorIngress::production(
        services
            .extension_ingress_parts()
            .expect("composition built the generic ingress"),
    );

    let body = json!({
        "update_id": 501,
        "message": {
            "message_id": 11,
            "date": 1710000000,
            "text": "please reply through the coordinator",
            "from": {"id": 9911, "is_bot": false, "first_name": "Ada"},
            "chat": {"id": 8675309, "type": "private"}
        }
    })
    .to_string();
    let evidence = ironclaw_product::auth::mark_shared_secret_header_verified(
        "X-Telegram-Bot-Api-Secret-Token".to_string(),
        TELEGRAM_INSTALLATION,
    );
    // Pre-resolve through the SAME binding service the production-registered
    // sink resolves with, so the scripted gateway lands on the exact scope
    // the admitted run executes under.
    let vendor_scope = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &evidence,
        &body,
    )
    .await;
    let paused_gateway = Arc::new(PausedReplyGateway::new(TELEGRAM_REPLY));
    inbound.register_scope_gateway_for_test(
        vendor_scope.clone(),
        Arc::clone(&paused_gateway) as Arc<dyn HostManagedModelGateway>,
    );

    let send_message_count_before_rejected_update = inbound
        .captured_network_requests_for_test()
        .iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .count();

    // A wrong shared secret is rejected on the wire before any admission —
    // the production secrets port resolved the CONFIGURED webhook secret and
    // the constant-time compare failed.
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &body,
            vec![("X-Telegram-Bot-Api-Secret-Token", "wrong".to_string())],
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    ingress.drain().await;
    assert_eq!(
        inbound
            .captured_network_requests_for_test()
            .iter()
            .filter(|request| request.url.ends_with("/sendMessage"))
            .count(),
        send_message_count_before_rejected_update,
        "a rejected update must not add a turn delivery; earlier pairing feedback is preserved"
    );

    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "the signed update must be accepted");

    // The model is deliberately paused so the generic observer must surface
    // a working indicator through the real Telegram adapter before the final
    // reply exists.
    for _ in 0..200 {
        if inbound
            .captured_network_requests_for_test()
            .iter()
            .any(|request| {
                request.url.ends_with("/sendMessage")
                    && String::from_utf8_lossy(&request.body).contains("Ironclaw is thinking...")
            })
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let requests = inbound.captured_network_requests_for_test();
    let working = requests
        .iter()
        .find(|request| {
            request.url.ends_with("/sendMessage")
                && String::from_utf8_lossy(&request.body).contains("Ironclaw is thinking...")
        })
        .expect("a running Telegram turn must post the generic working indicator");
    assert!(String::from_utf8_lossy(&working.body).contains("8675309"));

    let run_id = paused_gateway.wait_for_run_id().await;
    paused_gateway.release();
    ingress.drain().await;
    let coordinator = inbound.turn_coordinator_for_test();
    wait_for_run_status_in_scope(&coordinator, &vendor_scope, run_id, TurnStatus::Completed).await;
    assert_delivered_attempt(services, &vendor_scope).await;
    event_router.wait_until_run_idle(run_id).await;

    // Wire seam: the coordinated reply reached sendMessage on the Bot API
    // with the token substituted host-side. #6520 delivery is event-driven,
    // so poll the wire with the file's bounded deadline instead of a single
    // post-idle snapshot (the send and its cleanup can land moments after
    // the router reports idle).
    let wire_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let (requests, send_message_position) = loop {
        let requests = inbound.captured_network_requests_for_test();
        let matched = requests
            .iter()
            .position(|request| {
                request.url.ends_with("/sendMessage")
                    && String::from_utf8_lossy(&request.body).contains(TELEGRAM_REPLY)
            })
            .filter(|_| {
                requests
                    .iter()
                    .any(|request| request.url.ends_with("/deleteMessage"))
            });
        if let Some(position) = matched {
            break (requests, position);
        }
        assert!(
            tokio::time::Instant::now() < wire_deadline,
            "sendMessage with the reply must land on the wire; got {:?}",
            requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let send_message = &requests[send_message_position];
    assert_eq!(
        send_message.url,
        format!("https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendMessage")
    );
    assert!(
        String::from_utf8_lossy(&send_message.body).contains("8675309"),
        "the reply must target the originating chat"
    );
    let delete_message = requests
        .iter()
        .find(|request| request.url.ends_with("/deleteMessage"))
        .expect("the final reply must retract the Telegram working indicator");
    let delete_body: serde_json::Value =
        serde_json::from_slice(&delete_message.body).expect("deleteMessage body is JSON");
    assert_eq!(delete_body["chat_id"], "8675309");
    assert_eq!(
        delete_body["message_id"], 4242,
        "cleanup uses the authoritative message_id returned by sendMessage"
    );

    // Updating the authorized manifest group refreshes every active consumer.
    // Activation-time values such as Telegram's webhook URL therefore take
    // effect without reintroducing a caller-visible configure/activate action.
    let updated_url = "https://hooks.example.test/webhooks/extensions/telegram/updates-v2";
    configure_admin_group(
        &group,
        "extension.telegram",
        1,
        json!([
            {"handle": "telegram_webhook_url", "value": updated_url},
            {"handle": "telegram_bot_token", "value": TELEGRAM_BOT_TOKEN},
            {"handle": "telegram_webhook_secret", "value": TELEGRAM_WEBHOOK_SECRET},
            {"handle": "bot_username", "value": "itest_delivery_bot"}
        ]),
    )
    .await;
    let requests = inbound.captured_network_requests_for_test();
    let set_webhook_calls: Vec<_> = requests
        .iter()
        .filter(|request| request.url.ends_with("/setWebhook"))
        .collect();
    assert!(
        set_webhook_calls.len() >= 2,
        "admin configuration refresh must re-run the activation hook; got {} setWebhook calls",
        set_webhook_calls.len()
    );
    let last_set_webhook = set_webhook_calls
        .last()
        .expect("at least one setWebhook call");
    assert!(
        String::from_utf8_lossy(&last_set_webhook.body).contains(updated_url),
        "the refreshed adapter must register the new webhook URL"
    );
}

/// §5.5 WebGeneratedCode pairing on the generic route (the P2 seam, DEL-10
/// shape): with telegram's binary-parity account-setup descriptor declared,
/// verified inbound actors resolve through the generic identity bindings and
/// an unbound DM fails closed into the connect nudge instead of inheriting
/// the operator. A code minted web-side (production pairing service — the
/// same instance the pairing routes and the connection facade hold) is
/// consumed from the verified webhook (`/start <code>`), binding the sender:
/// the durable pairing state flips to connected and the next plain DM admits
/// a turn whose scope subject IS the paired user, with the reply coordinated
/// back over `sendMessage`. Storage-mode-invariant semantics ride the
/// libsql case; the sibling delivery proof covers the backend matrix.
#[rstest]
#[case::libsql(StorageMode::LibSql)]
#[tokio::test]
async fn unbound_telegram_actor_pairs_via_web_minted_code_then_turns_attribute_to_the_paired_user(
    #[case] storage: StorageMode,
) {
    // Boxed like `telegram_update_becomes_a_turn_and_a_coordinated_reply`
    // above: inline, this journey's future overflows the 2 MiB test-thread
    // stack under llvm-cov instrumentation (main's Coverage lanes).
    Box::pin(
        unbound_telegram_actor_pairs_via_web_minted_code_then_turns_attribute_to_the_paired_user_impl(storage),
    )
    .await;
}

async fn unbound_telegram_actor_pairs_via_web_minted_code_then_turns_attribute_to_the_paired_user_impl(
    storage: StorageMode,
) {
    let group = RebornIntegrationGroup::builder()
        .storage(storage)
        .extension_delivery()
        .await
        .expect("delivery group builds on this backend");
    let services = reborn_services(&group);

    let inbound = group
        .thread("conv-telegram-pairing-inbound")
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await
        .expect("inbound thread builds");

    let assembly = services
        .start_channel_host_assembly_for_test(ChannelHostAssemblyTestWiring {
            thread_service: inbound
                .thread_service_for_test()
                .expect("group thread service"),
            turn_coordinator: inbound.turn_coordinator_for_test(),
            run_delivery_events: group
                .run_delivery_events()
                .expect("delivery group wires the canonical run-delivery event router"),
            identity: ChannelHostIdentity {
                tenant_id: inbound.binding.tenant_id.clone(),
                agent_id: inbound.binding.agent_id.clone().expect("binding agent id"),
                project_id: inbound.binding.project_id.clone(),
                operator_user_id: inbound
                    .binding
                    .subject_user_id
                    .clone()
                    .expect("binding subject user id"),
            },
        })
        .expect("the production channel host assembly starts over the composed runtime");

    let lifecycle = group
        .thread("conv-telegram-pairing-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "telegram"}),
            ),
            RebornScriptedReply::text("installed and ready"),
        ])
        .build()
        .await
        .expect("telegram lifecycle thread builds");

    configure_admin_group(
        &group,
        "extension.telegram",
        0,
        json!([
            {"handle": "telegram_webhook_url", "value": "https://hooks.example.test/webhooks/extensions/telegram/updates"},
            {"handle": "telegram_bot_token", "value": TELEGRAM_BOT_TOKEN},
            {"handle": "telegram_webhook_secret", "value": TELEGRAM_WEBHOOK_SECRET},
            {"handle": "bot_username", "value": "itest_pairing_bot"}
        ]),
    )
    .await;

    let (install_run_id, _gate_ref) = lifecycle
        .submit_turn_until_auth_blocked("install telegram")
        .await
        .expect("unpaired Telegram install parks on its pairing requirement");

    // Bootstrap one caller-scoped account connection through the same generic
    // pairing service. Pairing resumes the exact install to active; this
    // journey keeps its focus on a second, unbound external actor afterward.
    let paired_user = inbound
        .binding
        .subject_user_id
        .clone()
        .expect("binding subject user id");
    let bootstrap_code = services
        .pairing_mint_for_test("telegram", &paired_user)
        .await
        .expect("setup-needed Telegram exposes pairing before readiness");
    assert_eq!(
        services
            .pairing_consume_for_test(
                "telegram",
                TELEGRAM_INSTALLATION,
                &bootstrap_code,
                ("user", "activation-bootstrap", None, "activation-bootstrap"),
                (
                    lifecycle.turn_coordinator_for_test(),
                    lifecycle.turn_state_store_for_test(),
                    inbound.binding.tenant_id.clone(),
                ),
            )
            .await
            .expect("bootstrap pairing completes")
            .as_ref(),
        Some(&paired_user)
    );
    lifecycle
        .wait_for_status(install_run_id, ironclaw_turns::TurnStatus::Completed)
        .await
        .expect("pairing continuation completes the exact install");

    let telegram_binding_service =
        wait_for_production_registration(&assembly, services, "telegram").await;
    let ingress = VendorIngress::production(
        services
            .extension_ingress_parts()
            .expect("composition built the generic ingress"),
    );
    let evidence = ironclaw_product::auth::mark_shared_secret_header_verified(
        "X-Telegram-Bot-Api-Secret-Token".to_string(),
        TELEGRAM_INSTALLATION,
    );

    let dm_body = |update_id: u64, chat_id: u64, text: &str| {
        json!({
            "update_id": update_id,
            "message": {
                "message_id": update_id + 10,
                "date": 1710000000,
                "text": text,
                "from": {"id": 424242, "is_bot": false, "first_name": "Pat"},
                "chat": {"id": chat_id, "type": "private"}
            }
        })
        .to_string()
    };
    let telegram_notices = services
        .pairing_connection_notices_for_test("telegram")
        .expect("the bundled manifest composes Telegram's pairing notices");

    // 1. Unbound plain DM: fail-closed actor resolution — no turn, no
    //    reply; the generic driver greets the 1:1 with the connect nudge.
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &dm_body(601, 515151, "hello, are you there?"),
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "vendor still gets its 2xx");
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &dm_body(602, 515151, "still there?"),
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "vendor still gets its 2xx");
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &dm_body(603, 616161, "hello from another chat"),
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "vendor still gets its 2xx");
    ingress.drain().await;
    let requests = inbound.captured_network_requests_for_test();
    let nudges: Vec<_> = requests
        .iter()
        .filter(|request| {
            request.url.ends_with("/sendMessage")
                && String::from_utf8_lossy(&request.body)
                    .contains(telegram_notices.connect_required.as_str())
        })
        .collect();
    assert_eq!(
        nudges.len(),
        2,
        "same-chat events share one 30-second nudge reservation while another chat gets its own"
    );
    assert!(
        nudges
            .iter()
            .any(|request| String::from_utf8_lossy(&request.body).contains("515151")),
        "the nudge must land in the sender's own chat"
    );
    assert!(
        nudges
            .iter()
            .any(|request| String::from_utf8_lossy(&request.body).contains("616161")),
        "a distinct conversation must receive its own nudge"
    );

    // 2. Web-side mint for the paired user (the production pairing service —
    //    the exact instance the pairing routes and connection facade hold).
    let code = services
        .pairing_mint_for_test("telegram", &paired_user)
        .await
        .expect("telegram's descriptor composes a pairing service; the channel is active");

    // 3. The verified webhook consumes the deep-link payload: the
    //    pre-admission gate services it (no turn) and binds the sender.
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &dm_body(604, 515151, &format!("/start {code}")),
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    ingress.drain().await;
    let requests = inbound.captured_network_requests_for_test();
    let paired_feedback = requests
        .iter()
        .find(|request| {
            request.url.ends_with("/sendMessage")
                && String::from_utf8_lossy(&request.body).contains(telegram_notices.paired.as_str())
        })
        .expect("successful pairing must post the descriptor's paired feedback");
    assert!(
        String::from_utf8_lossy(&paired_feedback.body).contains("515151"),
        "paired feedback must land in the code sender's conversation"
    );
    assert_eq!(
        services
            .pairing_connected_for_test("telegram", &paired_user)
            .await,
        Some(true),
        "consuming the minted code must durably connect the caller"
    );
    for intercepted_text in [
        "hello, are you there?",
        "still there?",
        "hello from another chat",
        code.as_str(),
    ] {
        assert!(
            inbound
                .assert_model_request_contains(intercepted_text)
                .await
                .is_err(),
            "unbound and pairing messages must not consume a scripted model reply: {intercepted_text}"
        );
    }

    // 4. The SAME actor's next plain DM now resolves through the pairing
    //    binding: a real turn admits under the paired user's scope and the
    //    reply coordinates back over sendMessage.
    let chat_body = dm_body(605, 515151, "what can you do now that we're paired?");
    let vendor_scope = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &evidence,
        &chat_body,
    )
    .await;
    assert_eq!(
        vendor_scope.explicit_owner_user_id(),
        Some(&paired_user),
        "post-pairing inbound must attribute to the paired user, not the operator fallback"
    );
    inbound.register_scope_gateway_for_test(
        vendor_scope.clone(),
        Arc::new(StaticReplyGateway(TELEGRAM_REPLY)),
    );
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &chat_body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    ingress.drain().await;
    // #6520 final-reply delivery is event-driven (RunDeliveryEventRouter), so
    // the send can land after ingress drain returns; poll the wire with the
    // same bounded deadline the file's other async seams use.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let requests = inbound.captured_network_requests_for_test();
        if requests.iter().any(|request| {
            request.url.ends_with("/sendMessage")
                && String::from_utf8_lossy(&request.body).contains(TELEGRAM_REPLY)
        }) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "sendMessage with the paired reply must land on the wire; got {:?}",
            requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_delivered_attempt(services, &vendor_scope).await;

    // 5. Exercise the real protected HTTP unpair handler. It must revoke both
    // identity and conversation-actor state, otherwise re-pairing this exact
    // Telegram chat would silently resurrect the old thread.
    let first_thread_id = vendor_scope.thread_id.clone();
    let pairing_mount = services
        .channel_pairing_route_mount_for_test()
        .expect("the composed runtime exposes the production pairing routes");
    let pairing_caller = ProductSurfaceCaller::new(
        inbound.binding.tenant_id.clone(),
        paired_user.clone(),
        inbound.binding.agent_id.clone(),
        inbound.binding.project_id.clone(),
    );
    let unpair_response = pairing_mount
        .router
        .layer(axum::Extension(pairing_caller))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webchat/v2/extensions/telegram/pairing/unpair")
                .body(Body::empty())
                .expect("unpair request"),
        )
        .await
        .expect("pairing router responds");
    assert_eq!(unpair_response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        services
            .pairing_connected_for_test("telegram", &paired_user)
            .await,
        Some(false),
        "HTTP unpair must revoke the caller's durable pairing"
    );

    // 6. Mint through the web-side pairing service and consume through the
    // real verified webhook again. No direct store/service mutation repairs
    // the actor binding in this journey.
    let repaired_code = services
        .pairing_mint_for_test("telegram", &paired_user)
        .await
        .expect("unpaired caller can mint a fresh code");
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &dm_body(606, 515151, &format!("/start {repaired_code}")),
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    ingress.drain().await;
    assert_eq!(
        services
            .pairing_connected_for_test("telegram", &paired_user)
            .await,
        Some(true),
        "verified webhook re-pair must restore the durable connection"
    );

    // 7. Resolving the same external actor/conversation now must allocate a
    // fresh thread. This assertion crosses the production conversation
    // binding seam and catches unpair implementations that delete only the
    // identity or DM target while leaving actor-thread state behind.
    let repaired_chat_body = dm_body(607, 515151, "do we have a fresh conversation now?");
    let repaired_scope = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &evidence,
        &repaired_chat_body,
    )
    .await;
    assert_ne!(
        &repaired_scope.thread_id, &first_thread_id,
        "unpair then re-pair must not resurrect the prior external-chat thread"
    );
    assert_eq!(repaired_scope.explicit_owner_user_id(), Some(&paired_user));
    inbound.register_scope_gateway_for_test(
        repaired_scope.clone(),
        Arc::new(StaticReplyGateway(TELEGRAM_REPLY)),
    );
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &repaired_chat_body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    ingress.drain().await;
    assert_delivered_attempt(services, &repaired_scope).await;
}
