//! Reborn integration test — generic outbound delivery through the REAL
//! coordinator (extension-runtime P5, §5.4 / OUT + DEL-10).
//!
//! Both proofs drive the FULL production inbound→outbound pipeline over the
//! composed runtime: a vendor-signed POST on the production ingress mount →
//! host-side recipe verification → the real channel adapter's normalization →
//! durable admission through the REAL `DefaultProductSurface` → a real turn
//! against a scripted model → the generic `RunDeliveryObserver` → the
//! factory-built `DeliveryCoordinator` (sole delivery-state writer, §5.4) →
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
//!   `[channel.config]` configure port (bot token + webhook secret into
//!   the scoped secret store, webhook URL into the durable installation
//!   store — zero test-only config injection), activates (`setWebhook`
//!   over recorded egress with host-side path-placeholder substitution of
//!   the configured token) — and the PRODUCTION channel host assembly
//!   (P6 S2) reconciles the activation into an ingress registration
//!   (dynamic `[channel.config]` verification secrets + per-extension
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
use ironclaw_product_adapters::{
    AdapterInstallationId, ChannelAdapter, InboundOutcome, ParsedProductInbound, ProductAdapterId,
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload, ProtocolAuthEvidence,
    UserMessagePayload, VerifiedInbound,
};
use ironclaw_product_workflow::{
    ChannelConnectionNoticePolicy, ConversationBindingService, ProductSurface,
    ResolveBindingRequest, RunDeliveryObserver, RunDeliveryServices, RunDeliverySettings,
};
use ironclaw_reborn_composition::{
    ChannelHostAssemblyTestWiring, ChannelHostIdentity, ChannelInboundSinkConfig,
    ChannelIngressRegistration, ExtensionIngressParts, GenericChannelHostAssembly,
    GenericChannelInboundSink, PostAdmissionObserver, RebornServices, StaticIngressSecrets,
    VerifiedEvidenceMint, extension_ingress_route_mount,
};
use ironclaw_turns::TurnScope;
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
}

impl PausedReplyGateway {
    fn new(reply: &'static str) -> Self {
        Self {
            reply,
            release: tokio::sync::Semaphore::new(0),
        }
    }

    fn release(&self) {
        self.release.add_permits(1);
    }
}

#[async_trait::async_trait]
impl HostManagedModelGateway for PausedReplyGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        let permit = self
            .release
            .acquire()
            .await
            .expect("paused test gateway semaphore remains open");
        permit.forget();
        Ok(HostManagedModelResponse::assistant_reply(self.reply))
    }
}

/// Post-admission observer that records every ack AND forwards to the REAL
/// generic run-delivery observer — the exact composition `serve` wires
/// (`RunDeliveryObserverAdapter`), plus recording so the tests can assert
/// admission outcomes.
struct RecordingForwardObserver {
    acks: Mutex<Vec<ProductInboundAck>>,
    errors: Mutex<Vec<String>>,
    inner: Arc<RunDeliveryObserver>,
}

impl RecordingForwardObserver {
    fn new(inner: Arc<RunDeliveryObserver>) -> Self {
        Self {
            acks: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            inner,
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
}

#[async_trait::async_trait]
impl PostAdmissionObserver for RecordingForwardObserver {
    async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        self.acks.lock().expect("acks lock").push(ack.clone());
        self.inner.observe_ack(envelope, ack).await;
    }

    async fn observe_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ironclaw_product_adapters::ProductAdapterError,
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
    services: &RebornServices,
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

/// Fast watcher pacing: the scripted model completes instantly; short polls
/// keep the drain-bounded observer wait snappy.
fn fast_delivery_settings() -> RunDeliverySettings {
    RunDeliverySettings {
        poll_interval: Duration::from_millis(20),
        max_wait: Duration::from_secs(30),
        ..RunDeliverySettings::default()
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
    let context = ironclaw_product_adapters::TrustedInboundContext::from_verified_evidence(
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
    mount: ironclaw_reborn_composition::PublicRouteMount,
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
        let surface = harness.product_workflow_for_test() as Arc<dyn ProductSurface>;
        let sink = Arc::new(GenericChannelInboundSink::new(ChannelInboundSinkConfig {
            adapter_id: ProductAdapterId::new(extension_id).expect("adapter id"),
            evidence,
            classifier: None,
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

/// Install + activate the REAL bundled slack package through the production
/// lifecycle tools (the same handshake `extension_runtime.rs` pins for
/// TOOL-7), so the coordinator's snapshot resolver sees an active slack
/// channel binding.
async fn activate_slack(group: &RebornIntegrationGroup) {
    let lifecycle = group
        .thread("conv-slack-delivery-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("installed"),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("activated"),
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
        .submit_turn("activate slack")
        .await
        .expect("slack activate completes");
    lifecycle
        .assert_tool_result_contains("\"activated\":true")
        .await
        .expect("slack activation reported success");
}

/// Assert the coordinator's ledger for `scope`: at least one attempt reached
/// terminal `Delivered`, and none is stranded mid-lifecycle
/// (`Prepared`/`Sending` — persist-before-egress must settle terminally).
async fn assert_delivered_attempt(services: &RebornServices, scope: &TurnScope) {
    let (outbound_store, _, _) = services
        .outbound_delivery_stores_for_test()
        .expect("outbound stores");
    let attempts = outbound_store
        .list_delivery_attempts(scope.clone())
        .await
        .expect("list delivery attempts");
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
    services: &RebornServices,
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

fn reborn_services(group: &RebornIntegrationGroup) -> &RebornServices {
    group
        .capability_harness()
        .expect("host-runtime capability harness")
        .reborn_services_for_test()
        .expect("composed reborn services")
}

async fn configure_admin_group(
    group: &RebornIntegrationGroup,
    group_id: &str,
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
    let scope = ResourceScope {
        // Admin configuration is deployment/tenant shared. The delivery
        // profile uses the default Reborn runtime identity; the separate
        // scripted turn harness intentionally lives under `tenant-itest` and
        // must not select which deployment receives operator configuration.
        tenant_id: ironclaw_host_api::TenantId::new("reborn-cli")
            .expect("delivery profile deployment tenant id"),
        user_id: operator_user_id.clone(),
        agent_id: Some(
            ironclaw_host_api::AgentId::new("reborn-cli-agent")
                .expect("delivery profile deployment agent id"),
        ),
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
        .host_runtime
        .as_ref()
        .expect("host runtime")
        .invoke_capability((
            context,
            capability_id,
            ResourceEstimate::default(),
            json!({
                "group_id": group_id,
                "expected_revision": 0,
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

async fn assert_extension_has_no_user_installation(services: &RebornServices, extension_id: &str) {
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
    services: &RebornServices,
    inbound: &RebornIntegrationHarness,
) -> Arc<GenericChannelHostAssembly> {
    services
        .start_channel_host_assembly_for_test(ChannelHostAssemblyTestWiring {
            thread_service: inbound
                .thread_service_for_test()
                .expect("group thread service"),
            turn_coordinator: inbound.turn_coordinator_for_test(),
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
            run_delivery_settings: fast_delivery_settings(),
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
    let assembly = start_channel_host_assembly(services, &inbound);
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
    let assembly = start_channel_host_assembly(services, &inbound);
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
    let notice = ChannelConnectionNoticePolicy::generic("Telegram");
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
#[tokio::test]
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
    let observer = Arc::new(RecordingForwardObserver::new(Arc::new(
        RunDeliveryObserver::with_settings(
            delivery_run_services(&inbound, services, "slack"),
            fast_delivery_settings(),
        ),
    )));
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
    let evidence = ironclaw_product_adapters::auth::mark_request_signature_verified(
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

    // Wire seam: the coordinated FinalReply reached chat.postMessage with the
    // bridged bot token injected host-side (the adapter never saw it).
    let requests = inbound.captured_network_requests_for_test();
    let post_message = requests
        .iter()
        .find(|request| {
            request.url.ends_with("/api/chat.postMessage")
                && String::from_utf8_lossy(&request.body).contains(SLACK_REPLY)
        })
        .unwrap_or_else(|| {
            panic!(
                "chat.postMessage with the reply must land on the wire; got {:?}",
                requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
            )
        });
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

    // Store seam: the coordinator (sole delivery-state writer) settled the
    // attempt terminally under the vendor conversation's scope.
    assert_delivered_attempt(services, &vendor_scope).await;
}

/// DEL-10: the bundled Telegram package — one manifest plus the adapter
/// crate, zero bespoke host code — installs through the production
/// lifecycle tool, is configured through the PRODUCTION `[channel.config]`
/// configure port, activates (`setWebhook` over recorded egress with the
/// CONFIGURED bot token substituted host-side into the URL path
/// placeholder), receives a signed update through the production router
/// mount, runs a real turn, delivers the reply through the generic
/// observer → REAL coordinator → `sendMessage`, and re-runs activation on
/// a config edit while Active (§6.5).
#[rstest]
#[case::libsql(StorageMode::LibSql)]
#[case::postgres(StorageMode::Postgres)]
#[tokio::test]
async fn telegram_update_becomes_a_turn_and_a_coordinated_reply(#[case] storage: StorageMode) {
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

    // Attach the PRODUCTION channel host assembly (P6 S2) over the composed
    // runtime. The harness supplies only its run-world services — the
    // group's shared turn runtime executes the admitted runs — while the
    // snapshot watch, ingress registry, `[channel.config]` secret storage,
    // durable workflow substrate, and delivery coordinator + outbound
    // stores are the production wiring. From here NOTHING registers the
    // telegram sink or observer manually.
    let assembly = services
        .start_channel_host_assembly_for_test(ChannelHostAssemblyTestWiring {
            thread_service: inbound
                .thread_service_for_test()
                .expect("group thread service"),
            turn_coordinator: inbound.turn_coordinator_for_test(),
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
            run_delivery_settings: fast_delivery_settings(),
        })
        .expect("the production channel host assembly starts over the composed runtime");

    // Install through the production lifecycle tool (same handshake as the
    // Slack proof), configure through the production port, THEN activate:
    // the real operator order, with zero test-only config injection.
    let lifecycle = group
        .thread("conv-telegram-delivery-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "telegram"}),
            ),
            RebornScriptedReply::text("installed"),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "telegram"}),
            ),
            RebornScriptedReply::text("activated"),
        ])
        .build()
        .await
        .expect("telegram lifecycle thread builds");
    lifecycle
        .submit_turn("install telegram")
        .await
        .expect("telegram install completes");
    lifecycle
        .assert_tool_result_contains("\"installed\":true")
        .await
        .expect("telegram install reported success");

    // The production configure surface: secrets land in the scoped secret
    // store (where channel egress resolves them), the webhook URL in the
    // durable installation store.
    let channel_config = services
        .channel_config_facade()
        .expect("the composed runtime exposes the channel-config configure port");
    let telegram_id = ironclaw_host_api::ExtensionId::new("telegram").expect("extension id");
    channel_config
        .save_values(
            &telegram_id,
            vec![
                (
                    "telegram_webhook_url".to_string(),
                    "https://hooks.example.test/webhooks/extensions/telegram/updates".to_string(),
                ),
                (
                    "telegram_bot_token".to_string(),
                    TELEGRAM_BOT_TOKEN.to_string(),
                ),
                (
                    "telegram_webhook_secret".to_string(),
                    TELEGRAM_WEBHOOK_SECRET.to_string(),
                ),
                ("bot_username".to_string(), "itest_delivery_bot".to_string()),
            ],
        )
        .await
        .expect("telegram configures through the production port");
    // §6.4 config completeness: every field reports provided, secrets as
    // presence only.
    let status = channel_config
        .field_status(&telegram_id)
        .await
        .expect("field status");
    assert_eq!(status.len(), 4, "{status:?}");
    // `bot_username` is optional pairing presentation (autofilled by the
    // setup-provisioning hook); the three transport fields are configured.
    assert!(
        status
            .iter()
            .filter(|field| field.name != "bot_username")
            .all(|field| field.provided),
        "all configured fields must report provided: {status:?}"
    );
    assert!(
        status
            .iter()
            .any(|field| field.name == "telegram_bot_token" && field.secret),
        "the bot token is a secret field: {status:?}"
    );

    let (activation_run_id, _activation_gate_ref) = lifecycle
        .submit_turn_until_auth_blocked("activate telegram")
        .await
        .expect("unpaired Telegram activation parks on its pairing requirement");
    let activation_state = lifecycle
        .wait_for_status(activation_run_id, ironclaw_turns::TurnStatus::BlockedAuth)
        .await
        .expect("Telegram activation remains blocked while the caller is unpaired");
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
    assert_eq!(
        installation.activation_state(),
        ironclaw_extensions::ExtensionActivationState::Installed,
        "unpaired activation must not publish or enable Telegram"
    );
    assert!(
        inbound
            .captured_network_requests_for_test()
            .iter()
            .all(|request| !request.url.ends_with("/setWebhook")),
        "the activation hook must not run before pairing"
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
        "the manifest template and configured username must survive activation gating"
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
        .expect("pairing continuation resumes the exact blocked activation");
    lifecycle
        .assert_tool_result_contains("\"activated\":true")
        .await
        .expect("telegram activation reported success");

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
    // model-visible activation result must not.
    let activation_output = lifecycle
        .tool_result_output("builtin.extension_activate")
        .await
        .expect("activation tool output");
    assert!(
        !activation_output
            .to_string()
            .contains(TELEGRAM_WEBHOOK_SECRET),
        "the webhook secret must not appear in model-visible tool output; got {activation_output}"
    );

    // The PRODUCTION assembly reconciled the activation into an ingress
    // registration: dynamic `[channel.config]` verification secrets, the
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
    let evidence = ironclaw_product_adapters::auth::mark_shared_secret_header_verified(
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

    paused_gateway.release();
    ingress.drain().await;

    // Wire seam: the coordinated reply reached sendMessage on the Bot API
    // with the token substituted host-side.
    let requests = inbound.captured_network_requests_for_test();
    let send_message = requests
        .iter()
        .find(|request| {
            request.url.ends_with("/sendMessage")
                && String::from_utf8_lossy(&request.body).contains(TELEGRAM_REPLY)
        })
        .unwrap_or_else(|| {
            panic!(
                "sendMessage with the reply must land on the wire; got {:?}",
                requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
            )
        });
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

    // Store seam: terminal Delivered attempt under the vendor scope.
    assert_delivered_attempt(services, &vendor_scope).await;

    // §6.5: editing `[channel.config]` while Active runs the automatic
    // deactivate → reactivate cycle through the REAL generic host — the
    // rebuilt adapter re-registers the webhook with the NEW URL.
    let updated_url = "https://hooks.example.test/webhooks/extensions/telegram/updates-v2";
    channel_config
        .save_values(
            &telegram_id,
            vec![("telegram_webhook_url".to_string(), updated_url.to_string())],
        )
        .await
        .expect("config edit while Active saves and reactivates");
    let requests = inbound.captured_network_requests_for_test();
    let set_webhook_calls: Vec<_> = requests
        .iter()
        .filter(|request| request.url.ends_with("/setWebhook"))
        .collect();
    assert!(
        set_webhook_calls.len() >= 2,
        "the reactivate cycle must re-run the activation hook; got {} setWebhook calls",
        set_webhook_calls.len()
    );
    let last_set_webhook = set_webhook_calls
        .last()
        .expect("at least one setWebhook call");
    assert!(
        String::from_utf8_lossy(&last_set_webhook.body).contains(updated_url),
        "the re-run activation must register the NEW webhook URL"
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
            run_delivery_settings: fast_delivery_settings(),
        })
        .expect("the production channel host assembly starts over the composed runtime");

    let lifecycle = group
        .thread("conv-telegram-pairing-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "telegram"}),
            ),
            RebornScriptedReply::text("installed"),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "telegram"}),
            ),
            RebornScriptedReply::text("activated"),
        ])
        .build()
        .await
        .expect("telegram lifecycle thread builds");
    lifecycle
        .submit_turn("install telegram")
        .await
        .expect("telegram install completes");

    let channel_config = services
        .channel_config_facade()
        .expect("the composed runtime exposes the channel-config configure port");
    let telegram_id = ironclaw_host_api::ExtensionId::new("telegram").expect("extension id");
    channel_config
        .save_values(
            &telegram_id,
            vec![
                (
                    "telegram_webhook_url".to_string(),
                    "https://hooks.example.test/webhooks/extensions/telegram/updates".to_string(),
                ),
                (
                    "telegram_bot_token".to_string(),
                    TELEGRAM_BOT_TOKEN.to_string(),
                ),
                (
                    "telegram_webhook_secret".to_string(),
                    TELEGRAM_WEBHOOK_SECRET.to_string(),
                ),
                ("bot_username".to_string(), "itest_pairing_bot".to_string()),
            ],
        )
        .await
        .expect("telegram configures through the production port");

    // Bootstrap one caller-scoped account connection through the same
    // generic pairing service before activation. The sibling Telegram test
    // drives the blocked-run continuation itself; this journey keeps its
    // existing focus on the post-activation behavior of a second, unbound
    // external actor.
    let paired_user = inbound
        .binding
        .subject_user_id
        .clone()
        .expect("binding subject user id");
    let bootstrap_code = services
        .pairing_mint_for_test("telegram", &paired_user)
        .await
        .expect("installed Telegram exposes pairing before activation");
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
        .submit_turn("activate telegram")
        .await
        .expect("telegram activate completes");

    let telegram_binding_service =
        wait_for_production_registration(&assembly, services, "telegram").await;
    let ingress = VendorIngress::production(
        services
            .extension_ingress_parts()
            .expect("composition built the generic ingress"),
    );
    let evidence = ironclaw_product_adapters::auth::mark_shared_secret_header_verified(
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
    let telegram_notices = ChannelConnectionNoticePolicy::generic("Telegram");

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
    let requests = inbound.captured_network_requests_for_test();
    requests
        .iter()
        .find(|request| {
            request.url.ends_with("/sendMessage")
                && String::from_utf8_lossy(&request.body).contains(TELEGRAM_REPLY)
        })
        .unwrap_or_else(|| {
            panic!(
                "sendMessage with the paired reply must land on the wire; got {:?}",
                requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
            )
        });
    assert_delivered_attempt(services, &vendor_scope).await;
}
