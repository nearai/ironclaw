// arch-exempt: large_file, whole-path channel delivery integration journeys, plan #6159
//! Reborn integration test — generic outbound delivery through the REAL
//! coordinator (extension-runtime P5, §5.4 / OUT + DEL-10).
//!
//! Both proofs drive the FULL production inbound→outbound pipeline over the
//! composed runtime: a vendor-signed POST on the production ingress mount →
//! host-side recipe verification → the real channel adapter's normalization →
//! durable admission through the REAL `DefaultProductSurface` → a real turn
//! against a scripted model → the canonical `RunDeliveryObserver` and
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
//! - The Slack proof: a signed threaded channel event yields a `FinalReply` coordinated
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

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use http_body_util::BodyExt;
use ironclaw_assistant::{RunDeliveryObserver, RunDeliveryServices, RunDeliverySettings};
use ironclaw_composition::{ChannelHostAssemblyTestWiring, RebornRuntime};
use ironclaw_extension_contracts::channel_adapter::{InboundOutcome, VerifiedInbound};
use ironclaw_extension_host::channel_host::{ChannelHostIdentity, GenericChannelHostAssembly};
use ironclaw_extension_host::extension_ingress::{
    ChannelInboundSinkConfig, ChannelIngressDrain, ChannelIngressRegistration,
    ExtensionIngressParts, GenericChannelInboundSink, PostAdmissionObserver,
    StaticIngressConfiguration, StaticIngressSecrets, VerifiedEvidenceMint,
    extension_ingress_route_mount,
};
use ironclaw_extension_host::ingress::{
    InboundAdmission, InboundAdmissionAck, InboundSink, InboundSinkError,
};
use ironclaw_host_api::product_adapter::auth::AuthRequirement;
use ironclaw_host_api::product_adapter::auth::ProtocolAuthEvidence;
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
use ironclaw_host_api::{
    action::NetworkPolicy,
    capability::{CapabilityGrant, CapabilitySet, EffectKind, GrantConstraints},
    ids::{CapabilityGrantId, CapabilityId, CorrelationId, ExtensionId, InvocationId, ProductKind},
    invocation::InvocationOrigin,
    mount::MountView,
    resource::{ResourceEstimate, ResourceScope},
    runtime::{RuntimeKind, TrustClass},
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::RuntimeCapabilityOutcome;
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_outbound::OutboundDeliveryStatus;
use ironclaw_product_contracts::binding::ProductBindingResolver;
use ironclaw_product_contracts::binding::ResolveBindingRequest;
use ironclaw_product_contracts::inbound::{
    ParsedProductInbound, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    UserMessagePayload,
};
use ironclaw_product_contracts::surface::ChannelInboundProductSurface;
use ironclaw_threads::FinalizedAssistantMessageByRunRequest;
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
const SLACK_CONNECT_REQUIRED: &str =
    "👋 Connect your Slack account in the IronClaw web app, then message me here again.";

const TELEGRAM_ROUTE: &str = "/webhooks/extensions/telegram/updates";
/// The PRODUCTION installation id: the lifecycle service mints installation
/// ids equal to the extension id, and the assembly's dynamic secrets port
/// reports that id as the verification candidate.
const TELEGRAM_INSTALLATION: &str = "telegram";
const TELEGRAM_WEBHOOK_SECRET: &str = "itest-telegram-webhook-secret";
const TELEGRAM_BOT_TOKEN: &str = "123456:itest-telegram-token";
const TELEGRAM_REPLY: &str = "Here is the coordinated Telegram reply.";
const TELEGRAM_CONNECT_REQUIRED: &str = "👋 Connect this Telegram account to the workspace bot from the Telegram extension in IronClaw, then message me again.";

struct UnexpectedAdmissionSink {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl InboundSink for UnexpectedAdmissionSink {
    async fn admit(
        &self,
        _admission: InboundAdmission,
    ) -> Result<InboundAdmissionAck, InboundSinkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(InboundAdmissionAck::Accepted)
    }
}

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

fn thread_scope_for_turn(scope: &TurnScope) -> ironclaw_threads::ThreadScope {
    ironclaw_threads::ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id: scope
            .agent_id
            .clone()
            .expect("vendor turn scope carries an agent id"),
        project_id: scope.project_id.clone(),
        owner_user_id: scope.explicit_owner_user_id().cloned(),
        mission_id: None,
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
        self.inner.observe_ack(envelope, ack).await;
    }

    async fn observe_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ironclaw_host_api::product_adapter::ProductAdapterError,
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
    let (outbound_store, route_store, communication_preferences, _, delivery_targets) = services
        .outbound_delivery_stores_for_test()
        .expect("composed runtime exposes the coordinator's outbound stores");
    let coordinator = services
        .delivery_coordinator()
        .expect("composition built the delivery coordinator");
    let fallback_notice_scope = TurnScope::new_with_owner(
        harness.binding.tenant_id.clone(),
        harness.binding.agent_id.clone(),
        harness.binding.project_id.clone(),
        ironclaw_host_api::ids::ThreadId::new(format!("{extension_id}-itest-channel-notices"))
            .expect("notice thread id"),
        Some(harness.binding.actor_user_id.clone()),
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
        project_filesystem: Arc::new(ironclaw_assistant::NoProjectFilesystem),
        delivery_targets,
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
#[allow(clippy::too_many_arguments)]
async fn preresolve_vendor_turn_scope(
    binding_service: &Arc<dyn ProductBindingResolver>,
    adapter: &dyn ironclaw_extension_contracts::channel_adapter::ChannelIngress,
    adapter_id: &str,
    installation_id: &str,
    non_secret_config: &[(String, String)],
    evidence: &ProtocolAuthEvidence,
    body: &str,
    // The channel's manifest `presentation.can_reply_in_threads`, mirroring
    // the value the host stamps from the resolved descriptor (#7377 made the
    // flag load-bearing for inbound placement): slack ships true, telegram
    // false. Must match the registered channel's manifest or this
    // pre-resolution can normalize a different conversation shape than the
    // admission path will.
    can_reply_in_threads: bool,
) -> (TurnScope, ironclaw_host_api::ids::UserId) {
    let ingress_egress =
        ironclaw_extension_contracts::test_support::conformance::ScriptedVendorServer::new(
            Arc::new(
                |_| ironclaw_extension_contracts::tool_adapter::RestrictedEgressResponse {
                    status: 503,
                    body: Vec::new(),
                },
            ),
        );
    let outcome = adapter
        .receive(
            VerifiedInbound {
                extension_id: adapter_id,
                installation_id,
                config: non_secret_config,
                body: body.as_bytes(),
                headers: &[],
                can_reply_in_threads,
            },
            &ingress_egress,
        )
        .await
        .expect("the vendor body must parse through the real adapter");
    let InboundOutcome::Messages(messages) = outcome else {
        panic!("the vendor body must normalize to messages");
    };
    let message = messages.first().expect("one normalized message");
    // Mirror of the sink's envelope assembly (`extension_ingress.rs::admit`).
    let context =
        ironclaw_product_contracts::inbound::TrustedInboundContext::from_verified_evidence(
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
        .resolve_binding(
            ResolveBindingRequest::from_envelope(&envelope)
                .expect("verified envelope binding request"),
        )
        .await
        .expect("vendor conversation binding resolves");
    (
        TurnScope::new_with_owner(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
            // Owner == actor under ephemeral-per-ping: the run's thread scope
            // is the acting user (the pinger). Mirrors production
            // `run_delivery::thread_scope_from_binding`.
            Some(binding.actor_user_id.clone()),
        ),
        binding.actor_user_id,
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
        let surface = harness.product_surface_for_test() as Arc<dyn ChannelInboundProductSurface>;
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
                configuration: Arc::new(StaticIngressConfiguration::default()),
                sink: sink.clone() as Arc<dyn ironclaw_extension_host::ingress::InboundSink>,
                drain: Some(sink as Arc<dyn ChannelIngressDrain>),
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
                "reactions:read",
                "reactions:write",
                "im:write",
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
    let (outbound_store, _, _, _, _) = services
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

fn assert_slack_thread_delivery_evidence(messages: &[serde_json::Value]) {
    let expected_conversation_id = "C777";
    let expected_thread_anchor = Some("1710000200.000050");
    let expected_count = 1;
    let matching = messages.iter().filter(|message| {
        message["channel"] == expected_conversation_id
            && message.get("thread_ts").and_then(serde_json::Value::as_str)
                == expected_thread_anchor
            && message["text"]
                .as_str()
                .is_some_and(|text| text.contains(SLACK_REPLY))
    });
    assert_eq!(
        matching.count(),
        expected_count,
        "the coordinated Slack reply must reach the exact channel thread once: {messages:?}"
    );
}

fn assert_telegram_topic_delivery_evidence(messages: &[serde_json::Value]) {
    let expected_conversation_id = "-1008675309";
    let expected_thread_anchor = Some(77);
    let expected_count = 1;
    let matching = messages.iter().filter(|message| {
        message["chat_id"] == expected_conversation_id
            && message
                .get("message_thread_id")
                .and_then(serde_json::Value::as_i64)
                == expected_thread_anchor
            && message["text"]
                .as_str()
                .is_some_and(|text| text.contains(TELEGRAM_REPLY))
    });
    assert_eq!(
        matching.count(),
        expected_count,
        "the coordinated Telegram reply must reach the exact forum topic once: {messages:?}"
    );
}

fn assert_telegram_chat_delivery_evidence(
    messages: &[serde_json::Value],
    expected_reply_to_message_id: i64,
) {
    let expected_conversation_id = "515151";
    let expected_thread_anchor: Option<&serde_json::Value> = None;
    let expected_count = 1;
    let matching = messages.iter().filter(|message| {
        message["chat_id"] == expected_conversation_id
            && message.get("message_thread_id") == expected_thread_anchor
            && message["text"]
                .as_str()
                .is_some_and(|text| text.contains(TELEGRAM_REPLY))
            // The reply must quote the prompting inbound message: without the
            // anchor, a reply landing after a newer user message reads as an
            // answer to the wrong prompt (#6644).
            && message["reply_to_message_id"] == expected_reply_to_message_id
    });
    assert_eq!(
        matching.count(),
        expected_count,
        "the coordinated Telegram reply must reach the exact unthreaded chat once, \
         anchored to the prompting message: {messages:?}"
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
) -> Arc<dyn ProductBindingResolver> {
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

async fn pair_telegram_bot_actor(
    ingress: &VendorIngress,
    services: &RebornRuntime,
    user_id: &ironclaw_host_api::ids::UserId,
    update_id: u64,
    external_actor_id: &str,
    conversation_id: &str,
) {
    let (code, deep_link, _expires_at) = services
        .pairing_issue_for_test("telegram", user_id)
        .await
        .expect("Telegram workspace-bot pairing code issues");
    assert!(
        deep_link
            .as_deref()
            .is_some_and(|link| link.contains(&format!("start={code}"))),
        "Telegram pairing issue must carry the manifest-derived bot deep link"
    );
    let actor_id = external_actor_id
        .parse::<i64>()
        .expect("Telegram actor id is numeric");
    let chat_id = conversation_id
        .parse::<i64>()
        .expect("Telegram conversation id is numeric");
    let body = json!({
        "update_id": update_id,
        "message": {
            "message_id": update_id + 10,
            "date": 1710000000,
            "text": format!("/start {code}"),
            "from": {"id": actor_id, "is_bot": false, "first_name": "Paired user"},
            "chat": {"id": chat_id, "type": "private"}
        }
    })
    .to_string();
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
    assert_eq!(
        status,
        StatusCode::OK,
        "the production Telegram ingress must accept the pairing command"
    );
    ingress.drain().await;
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
    let operator_user_id =
        ironclaw_host_api::ids::UserId::new("reborn-e2e-extension-lifecycle-tools")
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
    _group: &RebornIntegrationGroup,
    services: &RebornRuntime,
    inbound: &RebornIntegrationHarness,
) -> Arc<GenericChannelHostAssembly> {
    services
        .start_channel_host_assembly_for_test(ChannelHostAssemblyTestWiring {
            thread_service: inbound
                .thread_service_for_test()
                .expect("group thread service"),
            turn_coordinator: inbound.turn_coordinator_for_test(),
            run_delivery_settings: RunDeliverySettings::default(),
            identity: ChannelHostIdentity {
                tenant_id: inbound.binding.tenant_id.clone(),
                agent_id: inbound.binding.agent_id.clone().expect("binding agent id"),
                project_id: inbound.binding.project_id.clone(),
                operator_user_id: inbound.binding.actor_user_id.clone(),
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
    let captured_requests = inbound.captured_network_requests_for_test();
    assert!(
        captured_requests.iter().any(|request| {
            request.url.ends_with("/api/chat.postMessage")
                && String::from_utf8_lossy(&request.body).contains(SLACK_CONNECT_REQUIRED)
        }),
        "the unconnected Slack DM must receive the manifest/generic connect notice; captured request bodies: {:?}",
        captured_requests
            .iter()
            .map(|request| (&request.url, String::from_utf8_lossy(&request.body)))
            .collect::<Vec<_>>()
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
    assert!(
        inbound
            .captured_network_requests_for_test()
            .iter()
            .any(|request| {
                request.url.ends_with("/sendMessage")
                    && String::from_utf8_lossy(&request.body).contains(TELEGRAM_CONNECT_REQUIRED)
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

#[tokio::test]
async fn telegram_identity_configuration_errors_are_retryable_on_the_real_router_path() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let services = reborn_services(&group);
    let parts = services
        .extension_ingress_parts()
        .expect("composition built generic ingress");
    let ingress = VendorIngress::production(parts.clone());
    let sink_calls = Arc::new(AtomicUsize::new(0));
    let body = json!({
        "update_id": 7101,
        "message": {
            "message_id": 7111,
            "date": 1710000000,
            "text": "configuration must be ready before this can be admitted",
            "from": {"id": 710710, "is_bot": false, "first_name": "Pat"},
            "chat": {"id": 710710, "type": "private"}
        }
    })
    .to_string();

    let configurations = [
        Vec::new(),
        vec![(
            ironclaw_telegram_extension::TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            "configured_identity".to_string(),
        )],
    ];
    let mut responses = Vec::new();
    for config in configurations {
        parts.registry.register(
            "telegram",
            ChannelIngressRegistration {
                secrets: Arc::new(StaticIngressSecrets::new(vec![
                    ironclaw_extension_host::ingress::VerificationCandidate {
                        installation_id: TELEGRAM_INSTALLATION.to_string(),
                        secret: TELEGRAM_WEBHOOK_SECRET.as_bytes().to_vec(),
                    },
                ])),
                configuration: Arc::new(StaticIngressConfiguration::new(config)),
                sink: Arc::new(UnexpectedAdmissionSink {
                    calls: Arc::clone(&sink_calls),
                }),
                drain: None,
            },
        );
        responses.push(
            ingress
                .post_with_body(
                    TELEGRAM_ROUTE,
                    &body,
                    vec![(
                        "X-Telegram-Bot-Api-Secret-Token",
                        TELEGRAM_WEBHOOK_SECRET.to_string(),
                    )],
                )
                .await,
        );
    }

    assert_eq!(
        responses
            .iter()
            .map(|(status, _)| *status)
            .collect::<Vec<_>>(),
        vec![
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::SERVICE_UNAVAILABLE
        ],
        "missing and invalid host identity configuration must be retryable"
    );
    assert!(
        responses
            .iter()
            .all(|(_, body)| body.contains("temporarily_unavailable")),
        "configuration failures must not be reported as malformed vendor payloads: {responses:?}"
    );
    assert_eq!(
        sink_calls.load(Ordering::SeqCst),
        0,
        "host configuration failure must stop before durable admission"
    );
}

/// The Slack outbound proof (OUT-1/2/5 + ING-11 read half): a signed threaded
/// channel
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
    let delivery_services = delivery_run_services(&inbound, services, "slack");
    let observer = Arc::new(RecordingForwardObserver::new(Arc::new(
        RunDeliveryObserver::new(delivery_services),
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
            "type": "app_mention",
            "user": "U777",
            "channel": "C777",
            "text": "<@UBOT> please reply through the coordinator",
            "thread_ts": "1710000200.000050",
            "ts": "1710000300.000100"
        }
    })
    .to_string();
    // The run's scope is the vendor conversation's binding, not this harness
    // thread's — register its scripted model before the POST admits the turn.
    // `test_verified` is the `test-support` seam standing in for the ingress
    // verifier: minting is witness-gated (PROPOSAL §11.2.5) and the harness
    // holds no `VerifiedInboundGrant`. Value-identical to the pre-WS1.5
    // `mark_request_signature_verified` call this replaced.
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::RequestSignature {
            header_name: "X-Slack-Signature".to_string(),
            timestamp_header_name: Some("X-Slack-Request-Timestamp".to_string()),
        },
        SLACK_INSTALLATION,
    );
    let slack_binding_service = inbound
        .binding_service_for_test()
        .expect("group binding service");
    let (vendor_scope, vendor_actor_user_id) = preresolve_vendor_turn_scope(
        &slack_binding_service,
        &ironclaw_slack_extension::SlackChannelAdapter,
        "slack",
        SLACK_INSTALLATION,
        &[],
        &evidence,
        &body,
        true,
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
        "the signed threaded channel message must be admitted as a turn (errors: {:?})",
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
        actor.user_id, vendor_actor_user_id,
        "the admitted Slack run actor must remain the normalized external account"
    );
    // Pin changed with the run-acts-as-invoker ruling: the shared route's
    // thread is owned by the PAIRED ACTOR who invoked it, not a configured
    // subject account.
    assert_eq!(
        vendor_scope.explicit_owner_user_id(),
        Some(&vendor_actor_user_id),
        "the shared Slack route's thread must be owned by the invoking actor"
    );
    let durable_reply = inbound
        .thread_service_for_test()
        .expect("group thread service")
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: thread_scope_for_turn(&vendor_scope),
            thread_id: vendor_scope.thread_id.clone(),
            turn_run_id: run_id.to_string(),
        })
        .await
        .expect("Slack thread history remains readable")
        .expect("Slack reply is durable");
    assert!(
        durable_reply
            .content
            .as_deref()
            .is_some_and(|content| content.contains(SLACK_REPLY)),
        "durable Slack reply must retain important content: {durable_reply:?}"
    );
    assert_delivered_attempt(services, &vendor_scope).await;

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
    let posted_messages = requests
        .iter()
        .filter(|request| request.url.ends_with("/api/chat.postMessage"))
        .map(|request| {
            serde_json::from_slice(&request.body).expect("Slack chat.postMessage body is JSON")
        })
        .collect::<Vec<_>>();
    assert_slack_thread_delivery_evidence(&posted_messages);
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
            run_delivery_settings: RunDeliverySettings::default(),
            identity: ChannelHostIdentity {
                tenant_id: inbound.binding.tenant_id.clone(),
                agent_id: inbound.binding.agent_id.clone().expect("binding agent id"),
                project_id: inbound.binding.project_id.clone(),
                operator_user_id: inbound.binding.actor_user_id.clone(),
            },
        })
        .expect("the production channel host assembly starts over the composed runtime");

    // Admin bot configuration is a separate tenant axis and is valid before
    // any user installs the channel. Workspace-bot activation and generated
    // code pairing must complete without MTProto deployment credentials or a
    // caller-owned personal-account credential.
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

    configure_admin_group(
        &group,
        "extension.telegram",
        0,
        json!([
            {"handle": "telegram_bot_token", "value": TELEGRAM_BOT_TOKEN},
            {"handle": "telegram_webhook_secret", "value": TELEGRAM_WEBHOOK_SECRET},
            {"handle": "telegram_webhook_url", "value": "https://hooks.example.test/webhooks/extensions/telegram/updates"},
            {"handle": "bot_username", "value": "itest_delivery_bot"},
            // Deliberately NO admission-related config: shared-conversation
            // admission is presence-based, so the supergroup this scenario
            // drives is served because the bot received its update through
            // the authenticated webhook — there is no allowlist. The served
            // supergroup turn below is the presence pin.
        ]),
    )
    .await;

    let paired_user = inbound.binding.actor_user_id.clone();
    lifecycle
        .submit_turn("install telegram")
        .await
        .expect("Telegram installs without requiring a personal device link");
    let telegram_binding_service =
        wait_for_production_registration(&assembly, services, "telegram").await;
    lifecycle
        .assert_tool_invoked("builtin.extension_install")
        .await
        .expect("the natural-language install turn invokes extension installation");
    let installation_store = services
        .extension_installation_store_for_test()
        .expect("extension delivery profile carries the lifecycle store");
    let installation_id =
        ironclaw_extension_registry::ExtensionInstallationId::new(TELEGRAM_INSTALLATION)
            .expect("Telegram installation id");
    let installation = installation_store
        .get_installation(&installation_id)
        .await
        .expect("Telegram installation state reads")
        .expect("Telegram installation exists after activation");
    assert!(installation.owner().visible_to(&paired_user));
    let ingress = VendorIngress::production(
        services
            .extension_ingress_parts()
            .expect("composition built the generic ingress"),
    );
    pair_telegram_bot_actor(&ingress, services, &paired_user, 500, "424242", "424242").await;
    let channel_connection = group
        .channel_connection()
        .expect("delivery group composes production channel connection");
    assert!(
        channel_connection
            .caller_channel_connected("telegram", &paired_user)
            .await
            .expect("Telegram connection state reads"),
        "workspace-bot pairing must be Telegram's channel-connected signal"
    );

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
    lifecycle
        .assert_conversation_history_lacks(TELEGRAM_WEBHOOK_SECRET)
        .await
        .expect("the webhook secret must not appear in the model-visible transcript");

    let body = json!({
        "update_id": 501,
        "message": {
            "message_id": 11,
            "message_thread_id": 77,
            "date": 1710000000,
            "text": "@itest_delivery_bot please reply through the coordinator",
            "entities": [{"type": "mention", "offset": 0, "length": 19}],
            "from": {"id": 424242, "is_bot": false, "first_name": "Ada"},
            "chat": {"id": -1008675309_i64, "type": "supergroup"}
        }
    })
    .to_string();
    // Same `test-support` seam as above; value-identical to the pre-WS1.5
    // `mark_shared_secret_header_verified` call this replaced.
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".to_string(),
        },
        TELEGRAM_INSTALLATION,
    );
    // Pre-resolve through the SAME binding service the production-registered
    // sink resolves with, so the scripted gateway lands on the exact scope
    // the admitted run executes under.
    let (vendor_scope, vendor_actor_user_id) = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &[(
            ironclaw_telegram_extension::TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            "itest_delivery_bot".to_string(),
        )],
        &evidence,
        &body,
        false,
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
    // The model is paused, so the first `/sendMessage` AFTER the baseline is the
    // working indicator (its copy varies per run, so match on the call, not the
    // words — content is pinned in the assistant's prompt unit test). Selecting
    // past the baseline avoids matching earlier pairing-feedback traffic.
    for _ in 0..200 {
        let send_message_count = inbound
            .captured_network_requests_for_test()
            .iter()
            .filter(|request| request.url.ends_with("/sendMessage"))
            .count();
        if send_message_count > send_message_count_before_rejected_update {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let requests = inbound.captured_network_requests_for_test();
    let working = requests
        .iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .nth(send_message_count_before_rejected_update)
        .expect("a running Telegram turn must post the generic working indicator");
    let working_body: serde_json::Value =
        serde_json::from_slice(&working.body).expect("working sendMessage body is JSON");
    assert_eq!(working_body["chat_id"], "-1008675309");
    assert_eq!(
        working_body["message_thread_id"], 77,
        "the working indicator must stay inside the originating forum topic"
    );

    let run_id = paused_gateway.wait_for_run_id().await;
    paused_gateway.release();
    ingress.drain().await;
    let coordinator = inbound.turn_coordinator_for_test();
    wait_for_run_status_in_scope(&coordinator, &vendor_scope, run_id, TurnStatus::Completed).await;
    let completed = coordinator
        .get_run_state(GetRunStateRequest {
            scope: vendor_scope.clone(),
            run_id,
        })
        .await
        .expect("completed Telegram topic run remains readable");
    let actor = completed
        .actor
        .as_ref()
        .expect("completed Telegram topic run actor");
    assert_eq!(
        actor.user_id, vendor_actor_user_id,
        "the Telegram topic run must retain the normalized external account actor"
    );
    assert_eq!(
        vendor_scope.explicit_owner_user_id(),
        Some(&paired_user),
        "the Telegram topic's thread must be owned by the invoking linked actor"
    );
    let durable_reply = inbound
        .thread_service_for_test()
        .expect("group thread service")
        .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
            scope: thread_scope_for_turn(&vendor_scope),
            thread_id: vendor_scope.thread_id.clone(),
            turn_run_id: run_id.to_string(),
        })
        .await
        .expect("Telegram topic thread history remains readable")
        .expect("Telegram topic reply is durable");
    assert!(
        durable_reply
            .content
            .as_deref()
            .is_some_and(|content| content.contains(TELEGRAM_REPLY)),
        "durable Telegram topic reply must retain important content: {durable_reply:?}"
    );
    assert_delivered_attempt(services, &vendor_scope).await;
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
    let sent_messages: Vec<serde_json::Value> = requests
        .iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .map(|request| {
            serde_json::from_slice(&request.body).expect("Telegram sendMessage body is JSON")
        })
        .collect::<Vec<_>>();
    assert_telegram_topic_delivery_evidence(&sent_messages);
    let delete_message = requests
        .iter()
        .find(|request| request.url.ends_with("/deleteMessage"))
        .expect("the final reply must retract the Telegram working indicator");
    let delete_body: serde_json::Value =
        serde_json::from_slice(&delete_message.body).expect("deleteMessage body is JSON");
    assert_eq!(delete_body["chat_id"], "-1008675309");
    assert_eq!(
        delete_body["message_id"], 4242,
        "cleanup uses the authoritative message_id returned by sendMessage"
    );

    // ── Second participant (#7377 run-acts-as-invoker): the same supergroup
    // topic is ONE shared canonical thread. A SECOND paired user's mention
    // resolves the SAME thread through the same production binding service,
    // their run acts as THEM, and their reply stays anchored in the topic.
    let second_user =
        ironclaw_host_api::ids::UserId::new("user-telegram-bravo").expect("second user id");
    installation_store
        .activate_membership(&installation_id, &second_user)
        .await
        .expect("second user joins the Telegram installation membership");
    // The second user independently pairs their verified Telegram bot identity.
    // The generated code binds the actor; personal-account device linking is a
    // separate credential path.
    pair_telegram_bot_actor(&ingress, services, &second_user, 549, "9912", "9912").await;

    let second_topic_body = json!({
        "update_id": 550,
        "message": {
            "message_id": 42,
            "message_thread_id": 77,
            "date": 1710000100,
            "text": "@itest_delivery_bot bravo follows up in the topic",
            "entities": [{"type": "mention", "offset": 0, "length": 19}],
            "from": {"id": 9912, "is_bot": false, "first_name": "Bea"},
            "chat": {"id": -1008675309_i64, "type": "supergroup"}
        }
    })
    .to_string();
    // Ephemeral-per-ping (#7397) at the binding seam: the second participant's
    // mention mints its OWN pinger-owned ephemeral thread — a DISTINCT scope
    // from the first participant's, owned by the second actor (owner == actor),
    // never the first binder. Their run still acts as THEM and the reply still
    // anchors in the same forum topic (asserted below).
    let (second_scope, second_actor_user_id) = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &[(
            ironclaw_telegram_extension::TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            "itest_delivery_bot".to_string(),
        )],
        &evidence,
        &second_topic_body,
        false,
    )
    .await;
    assert_ne!(
        second_scope.thread_id, vendor_scope.thread_id,
        "each ping mints its own ephemeral thread — the second participant does not join the first's"
    );
    assert_eq!(
        second_scope.explicit_owner_user_id(),
        Some(&second_user),
        "the second participant's ephemeral thread is owned by the second actor (owner == actor)"
    );
    assert_eq!(second_actor_user_id, second_user);
    assert_ne!(
        second_actor_user_id, vendor_actor_user_id,
        "the second participant is a genuinely distinct canonical user"
    );
    // The second turn executes under its OWN ephemeral scope, so the scripted
    // gateway must be registered for that scope (the first gateway only serves
    // the first participant's thread).
    let second_gateway = Arc::new(PausedReplyGateway::new(TELEGRAM_REPLY));
    inbound.register_scope_gateway_for_test(
        second_scope.clone(),
        Arc::clone(&second_gateway) as Arc<dyn HostManagedModelGateway>,
    );

    let reply_sends_before = inbound
        .captured_network_requests_for_test()
        .iter()
        .filter(|request| {
            request.url.ends_with("/sendMessage")
                && String::from_utf8_lossy(&request.body).contains(TELEGRAM_REPLY)
        })
        .count();
    // The second participant's own scope-registered gateway serves its run:
    // pre-release one permit so the second turn completes unpaused.
    second_gateway.release();
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &second_topic_body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "the second participant is admitted");
    ingress.drain().await;
    let second_run_id = second_gateway.wait_for_run_id().await;
    assert_ne!(
        second_run_id, run_id,
        "the second participant's run is distinct from the first participant's"
    );
    wait_for_run_status_in_scope(
        &coordinator,
        &second_scope,
        second_run_id,
        TurnStatus::Completed,
    )
    .await;
    let second_run = coordinator
        .get_run_state(GetRunStateRequest {
            scope: second_scope.clone(),
            run_id: second_run_id,
        })
        .await
        .expect("second participant's completed run remains readable");
    assert_eq!(
        second_run.actor.as_ref().expect("second run actor").user_id,
        second_user,
        "the second participant's run acts as its own invoker",
    );
    let wire_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let bravo_reply = loop {
        let replies: Vec<serde_json::Value> = inbound
            .captured_network_requests_for_test()
            .iter()
            .filter(|request| {
                request.url.ends_with("/sendMessage")
                    && String::from_utf8_lossy(&request.body).contains(TELEGRAM_REPLY)
            })
            .map(|request| {
                serde_json::from_slice(&request.body).expect("Telegram sendMessage body is JSON")
            })
            .collect();
        if replies.len() > reply_sends_before {
            break replies.last().cloned().expect("latest reply body");
        }
        assert!(
            tokio::time::Instant::now() < wire_deadline,
            "the second participant's coordinated reply must land on the wire"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(bravo_reply["chat_id"], "-1008675309");
    assert_eq!(
        bravo_reply["message_thread_id"], 77,
        "the second participant's reply stays anchored inside the same forum topic"
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

    // ── Channel attachment journey (relocated from the composition-resident
    // attachment_journey_tests): a document update on the production mount
    // fetches bytes through the manifest's path-constrained `getFile` +
    // `/file/bot{token}/` egress inside adapter receive and before durable admission,
    // lands them through
    // the canonical project-filesystem authority, and starts a byte-free turn
    // whose transcript message carries `/workspace/attachments/...` refs. A
    // transient provider failure occurs before admission (503), so the vendor
    // retry refetches; a duplicate replay after success refetches before the
    // product idempotency check but does not reland.
    let attachment_body = json!({
        "update_id": 502,
        "message": {
            "message_id": 12,
            "date": 1710000300,
            "caption": "review the attached report",
            "document": {
                "file_id": "doc-file-1",
                "file_unique_id": "doc-unique-1",
                "file_name": "report.pdf",
                "mime_type": "application/pdf",
                "file_size": 4
            },
            "from": {"id": 424242, "is_bot": false, "first_name": "Ada"},
            "chat": {"id": 8675309, "type": "private"}
        }
    })
    .to_string();
    // This private DM is a distinct provider conversation from the earlier
    // supergroup topic. Resolve and register its own model scope so the
    // transcript assertion cannot accidentally read the topic thread.
    let attachment_scope_body = json!({
        "update_id": 500,
        "message": {
            "message_id": 11,
            "date": 1710000299,
            "text": "prepare attachment scope",
            "from": {"id": 424242, "is_bot": false, "first_name": "Ada"},
            "chat": {"id": 8675309, "type": "private"}
        }
    })
    .to_string();
    let (attachment_scope, attachment_actor_user_id) = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &[(
            ironclaw_telegram_extension::TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            "itest_delivery_bot".to_string(),
        )],
        &evidence,
        &attachment_scope_body,
        false,
    )
    .await;
    let attachment_gateway = Arc::new(PausedReplyGateway::new("Attachment received."));
    inbound.register_scope_gateway_for_test(
        attachment_scope.clone(),
        Arc::clone(&attachment_gateway) as Arc<dyn HostManagedModelGateway>,
    );
    attachment_gateway.release();
    let get_file_urls = |requests: &[ironclaw_network::NetworkHttpRequest]| {
        requests
            .iter()
            .filter(|request| request.url.ends_with("/getFile"))
            .count()
    };
    let download_urls = |requests: &[ironclaw_network::NetworkHttpRequest]| {
        requests
            .iter()
            .filter(|request| request.url.contains("api.telegram.org/file/"))
            .count()
    };

    // First delivery: the scripted transient `getFile` failure occurs inside
    // adapter receive and surfaces a retryable 503 to the vendor — never a
    // durable admission attempt.
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &attachment_body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a retryable transfer failure must ask the vendor to redeliver"
    );
    ingress.drain().await;

    // Vendor redelivery refetches before admission — `getFile` on
    // the exact declared path and the download through the manifest's
    // `/file/bot{token}/` prefix, both with the token injected host-side —
    // then admission commits and the turn starts byte-free.
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &attachment_body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "the redelivered update is accepted");
    ingress.drain().await;
    let attachment_run_id = attachment_gateway.wait_for_run_id().await;
    let coordinator = inbound.turn_coordinator_for_test();
    wait_for_run_status_in_scope(
        &coordinator,
        &attachment_scope,
        attachment_run_id,
        TurnStatus::Completed,
    )
    .await;
    let attachment_run = coordinator
        .get_run_state(GetRunStateRequest {
            scope: attachment_scope.clone(),
            run_id: attachment_run_id,
        })
        .await
        .expect("completed Telegram attachment run remains readable");
    assert_eq!(
        attachment_run
            .actor
            .as_ref()
            .expect("attachment run actor")
            .user_id,
        attachment_actor_user_id
    );

    let requests = inbound.captured_network_requests_for_test();
    assert_eq!(
        get_file_urls(&requests),
        2,
        "the released attempt plus the successful retry each look the file up once"
    );
    assert!(requests.iter().any(|request| {
        request.url == format!("https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/getFile")
    }));
    assert_eq!(download_urls(&requests), 1);
    assert!(
        requests.iter().any(|request| {
            request.url
                == format!(
                    "https://api.telegram.org/file/bot{TELEGRAM_BOT_TOKEN}/documents/report.pdf"
                )
        }),
        "the byte download must ride the manifest's path-prefixed egress with the injected token"
    );

    // The accepted transcript message carries the canonical byte-free
    // workspace ref the agent's file tools resolve.
    let history = inbound
        .thread_service_for_test()
        .expect("group thread service")
        .list_thread_history(ironclaw_threads::ThreadHistoryRequest {
            scope: thread_scope_for_turn(&attachment_scope),
            thread_id: attachment_scope.thread_id.clone(),
        })
        .await
        .expect("vendor thread history");
    let attachment_messages: Vec<_> = history
        .messages
        .iter()
        .filter(|message| !message.attachments.is_empty())
        .collect();
    assert_eq!(
        attachment_messages.len(),
        1,
        "the failed receive and successful retry must produce one landed attachment"
    );
    let storage_key = attachment_messages[0].attachments[0]
        .storage_key
        .as_deref()
        .expect("landed attachment carries a canonical workspace ref");
    assert!(
        storage_key.starts_with("/workspace/attachments/"),
        "unexpected storage key {storage_key}"
    );

    // Duplicate replay after durable success still completes adapter receive
    // before the product idempotency check. It therefore refetches, but the
    // durable replay must neither rerun nor reland the attachment.
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &attachment_body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "duplicate replay is acknowledged");
    ingress.drain().await;
    let requests = inbound.captured_network_requests_for_test();
    assert_eq!(
        get_file_urls(&requests),
        3,
        "duplicate replay completes the adapter-owned lookup before dedupe"
    );
    assert_eq!(
        download_urls(&requests),
        2,
        "duplicate replay completes the adapter-owned download before dedupe"
    );
    let history = inbound
        .thread_service_for_test()
        .expect("group thread service")
        .list_thread_history(ironclaw_threads::ThreadHistoryRequest {
            scope: thread_scope_for_turn(&attachment_scope),
            thread_id: attachment_scope.thread_id.clone(),
        })
        .await
        .expect("vendor thread history after duplicate");
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|message| !message.attachments.is_empty())
            .count(),
        1,
        "a duplicate replay may refetch transient bytes but must not reland them"
    );

    // ── Outbound half: a final reply in a NEW conversation (same paired
    // actor, so the same project workspace) explicitly invokes the generic
    // reply-attachment capability for the landed file. Transcript finalization
    // seals that run-scoped intent into the assistant message; the coordinator
    // materializes the bytes through the real project-scoped reader and the
    // adapter delivers them natively as `sendDocument`.
    let outbound_body = json!({
        "update_id": 503,
        "message": {
            "message_id": 13,
            "date": 1710000400,
            "text": "send me the report back",
            "from": {"id": 424242, "is_bot": false, "first_name": "Ada"},
            "chat": {"id": 424242, "type": "private"}
        }
    })
    .to_string();
    let (outbound_scope, _) = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &[(
            ironclaw_telegram_extension::TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            "itest_delivery_bot".to_string(),
        )],
        &evidence,
        &outbound_body,
        false,
    )
    .await;
    group
        .register_scope_script_for_test(
            outbound_scope,
            "telegram-outbound-reply-attachment",
            [
                RebornScriptedReply::tool_call(
                    ironclaw_host_runtime::ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID,
                    json!({"path": storage_key}),
                ),
                RebornScriptedReply::text("Here is the report."),
            ],
        )
        .await
        .expect("outbound attachment scope uses the real scripted provider chain");
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &outbound_body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    ingress.drain().await;

    let wire_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let send_document = loop {
        let requests = inbound.captured_network_requests_for_test();
        if let Some(request) = requests
            .iter()
            .find(|request| request.url.ends_with("/sendDocument"))
        {
            break request.clone();
        }
        assert!(
            tokio::time::Instant::now() < wire_deadline,
            "sendDocument must land on the wire; got {:?}",
            requests.iter().map(|r| r.url.clone()).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(
        send_document.url,
        format!("https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendDocument")
    );
    let multipart = String::from_utf8_lossy(&send_document.body);
    assert!(
        multipart.contains("DATA"),
        "the delivered document must carry the landed workspace bytes"
    );
    // The landed segment is `<message_id>-<index>-report.pdf`; the delivered
    // filename is derived from that path segment.
    assert!(
        multipart.contains("report.pdf\""),
        "the delivered document keeps the landed filename; got {multipart}"
    );
    assert!(
        multipart.contains("424242"),
        "the document targets the replying conversation"
    );
    inbound
        .assert_tool_invoked(ironclaw_host_runtime::ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID)
        .await
        .expect("Telegram file delivery was sourced from the explicit reply-attachment tool");
}

/// Added with the run-acts-as-invoker ruling (#7377): presence admits the
/// supergroup, linked identity gates the RUN. A mention from a `from` id that
/// was never linked executes nothing; the manifest's fixed `connect_required` notice is
/// posted back INTO the conversation as a reply anchored on the sender's own
/// message (`reply_to_message_id` — telegram declares
/// `presentation.can_reply_in_threads = false`, so anchored quoting is its
/// in-place placement), and the wire carries exactly that one send.
#[tokio::test]
async fn telegram_unlinked_group_mention_gets_a_quoted_connect_notice() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("delivery group builds");
    let services = reborn_services(&group);
    let inbound = group
        .thread("conv-telegram-unpaired-group")
        .script([RebornScriptedReply::text("must stay unused")])
        .build()
        .await
        .expect("inbound thread builds");
    let assembly = start_channel_host_assembly(&group, services, &inbound);
    let _binding = wait_for_production_registration(&assembly, services, "telegram").await;
    let ingress = VendorIngress::production(
        services
            .extension_ingress_parts()
            .expect("composition built generic ingress"),
    );
    configure_admin_group(
        &group,
        "extension.telegram",
        0,
        json!([
            {"handle": "telegram_bot_token", "value": TELEGRAM_BOT_TOKEN},
            {"handle": "telegram_webhook_secret", "value": TELEGRAM_WEBHOOK_SECRET},
            {"handle": "telegram_webhook_url", "value": "https://hooks.example.test/webhooks/extensions/telegram/updates"},
            {"handle": "bot_username", "value": "itest_unpaired_bot"}
            // Deliberately NO admission-related config: the supergroup is
            // admitted by presence (the verified webhook delivering the
            // update IS the admission); only the SENDER's pairing is missing.
        ]),
    )
    .await;

    let message = "unpaired supergroup mention must not reach the agent";
    let body = json!({
        "update_id": 8101,
        "message": {
            "message_id": 8111,
            "message_thread_id": 88,
            "date": 1710000000,
            "text": format!("@itest_unpaired_bot {message}"),
            "entities": [{"type": "mention", "offset": 0, "length": 19}],
            "from": {"id": 424243, "is_bot": false, "first_name": "Uma"},
            "chat": {"id": -1008675310_i64, "type": "supergroup"}
        }
    })
    .to_string();
    let send_message_count_before = inbound
        .captured_network_requests_for_test()
        .iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .count();
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
        "the verified unpaired update is acknowledged: {response_body}"
    );
    ingress.drain().await;

    let sends: Vec<serde_json::Value> = inbound
        .captured_network_requests_for_test()
        .iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .map(|request| {
            serde_json::from_slice(&request.body).expect("Telegram sendMessage body is JSON")
        })
        .collect();
    assert_eq!(
        sends.len(),
        send_message_count_before + 1,
        "exactly one connect notice and nothing else crosses the wire: {sends:?}"
    );
    let nudge = sends.last().expect("one sendMessage recorded");
    assert!(
        nudge["text"]
            .as_str()
            .is_some_and(|text| text.contains(TELEGRAM_CONNECT_REQUIRED)),
        "the notice carries the manifest's connect_required copy: {nudge}"
    );
    assert_eq!(
        nudge["reply_to_message_id"], 8111,
        "the nudge quotes the unpaired sender's own message"
    );
    assert_eq!(nudge["chat_id"], "-1008675310");
    assert!(
        inbound
            .assert_model_request_contains(message)
            .await
            .is_err(),
        "the unpaired supergroup mention must not admit an agent turn"
    );
}

/// Telegram workspace-bot pairing on the generic ingress route: an unbound
/// verified DM fails closed into the connect nudge instead of inheriting the
/// operator. Consuming a caller-issued pairing code binds the verified bot
/// actor and admits the next plain DM as that IronClaw user, with its reply
/// coordinated over `sendMessage`. Disconnect removes that admission; a fresh
/// pairing code restores it. Storage-mode-invariant semantics ride the libSQL
/// case; the sibling delivery proof covers the backend matrix.
#[rstest]
#[case::libsql(StorageMode::LibSql)]
#[tokio::test]
async fn paired_telegram_bot_actor_turns_attribute_to_the_user_and_disconnect_revokes_admission(
    #[case] storage: StorageMode,
) {
    // Boxed like `telegram_update_becomes_a_turn_and_a_coordinated_reply`
    // above: inline, this journey's future overflows the 2 MiB test-thread
    // stack under llvm-cov instrumentation (main's Coverage lanes).
    Box::pin(
        paired_telegram_bot_actor_turns_attribute_to_the_user_and_disconnect_revokes_admission_impl(
            storage,
        ),
    )
    .await;
}

async fn paired_telegram_bot_actor_turns_attribute_to_the_user_and_disconnect_revokes_admission_impl(
    storage: StorageMode,
) {
    let group = RebornIntegrationGroup::builder()
        .storage(storage)
        .extension_delivery()
        .await
        .expect("delivery group builds on this backend");
    let services = reborn_services(&group);

    let inbound = group
        .thread("conv-telegram-linked-inbound")
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
            run_delivery_settings: RunDeliverySettings::default(),
            identity: ChannelHostIdentity {
                tenant_id: inbound.binding.tenant_id.clone(),
                agent_id: inbound.binding.agent_id.clone().expect("binding agent id"),
                project_id: inbound.binding.project_id.clone(),
                operator_user_id: inbound.binding.actor_user_id.clone(),
            },
        })
        .expect("the production channel host assembly starts over the composed runtime");

    let lifecycle = group
        .thread("conv-telegram-paired-lifecycle")
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
            {"handle": "telegram_bot_token", "value": TELEGRAM_BOT_TOKEN},
            {"handle": "telegram_webhook_secret", "value": TELEGRAM_WEBHOOK_SECRET},
            {"handle": "telegram_webhook_url", "value": "https://hooks.example.test/webhooks/extensions/telegram/updates"},
            {"handle": "bot_username", "value": "itest_linked_bot"},
        ]),
    )
    .await;
    let paired_user = inbound.binding.actor_user_id.clone();
    lifecycle
        .submit_turn("install telegram")
        .await
        .expect("Telegram installs without requiring a personal device link");
    let channel_connection = group
        .channel_connection()
        .expect("delivery group composes production channel connection");

    let telegram_binding_service =
        wait_for_production_registration(&assembly, services, "telegram").await;
    lifecycle
        .assert_tool_invoked("builtin.extension_install")
        .await
        .expect("the natural-language install turn invokes extension installation");
    let ingress = VendorIngress::production(
        services
            .extension_ingress_parts()
            .expect("composition built the generic ingress"),
    );
    // Same `test-support` seam as above; value-identical to the pre-WS1.5
    // `mark_shared_secret_header_verified` call this replaced.
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".to_string(),
        },
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
                && String::from_utf8_lossy(&request.body).contains(TELEGRAM_CONNECT_REQUIRED)
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

    // 2. Generated-code pairing supplies the verified Bot API actor identity.
    // No personal-account credential is created or consulted.
    pair_telegram_bot_actor(&ingress, services, &paired_user, 604, "424242", "515151").await;
    assert!(
        channel_connection
            .caller_channel_connected("telegram", &paired_user)
            .await
            .expect("connection state reads")
    );
    for intercepted_text in [
        "hello, are you there?",
        "still there?",
        "hello from another chat",
    ] {
        assert!(
            inbound
                .assert_model_request_contains(intercepted_text)
                .await
                .is_err(),
            "unbound messages must not consume a scripted model reply: {intercepted_text}"
        );
    }

    // 3. The SAME actor's next plain DM now resolves through the workspace-bot
    //    pairing: a real turn admits under the paired user's scope and the
    //    reply coordinates back over sendMessage.
    let chat_body = dm_body(605, 515151, "what can you do now that we're paired?");
    let (vendor_scope, _) = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &[(
            ironclaw_telegram_extension::TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            "itest_linked_bot".to_string(),
        )],
        &evidence,
        &chat_body,
        false,
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
    // #6520 final-reply delivery is observer-driven, so
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
    let delivered_messages = inbound
        .captured_network_requests_for_test()
        .into_iter()
        .filter(|request| request.url.ends_with("/sendMessage"))
        .map(|request| {
            serde_json::from_slice(&request.body).expect("Telegram sendMessage body is JSON")
        })
        .collect::<Vec<_>>();
    assert_telegram_chat_delivery_evidence(&delivered_messages, 615);
    assert_delivered_attempt(services, &vendor_scope).await;

    // 4. Disconnect through the same production connection service extension
    // removal uses. The old actor immediately loses admission, while the
    // generated-code service remains available for an explicit repair.
    channel_connection
        .disconnect_channel("telegram", &paired_user)
        .await
        .expect("Telegram disconnect completes");
    assert!(
        !channel_connection
            .caller_channel_connected("telegram", &paired_user)
            .await
            .expect("disconnected state reads")
    );
    let disconnected_text = "this must stay outside the agent after unlink";
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &dm_body(606, 515151, disconnected_text),
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    ingress.drain().await;
    assert!(
        inbound
            .assert_model_request_contains(disconnected_text)
            .await
            .is_err(),
        "an unlinked Telegram actor must not admit a turn"
    );

    // 5. Pairing the same verified bot actor with a fresh code restores
    // admission without linking a personal account.
    pair_telegram_bot_actor(&ingress, services, &paired_user, 610, "424242", "515151").await;

    // 6. The same external actor/conversation is admitted again through the
    // workspace-bot pairing and coordinated delivery remains healthy.
    let repaired_chat_body = dm_body(607, 515151, "are we connected again?");
    let (repaired_scope, _) = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &[(
            ironclaw_telegram_extension::TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            "itest_linked_bot".to_string(),
        )],
        &evidence,
        &repaired_chat_body,
        false,
    )
    .await;
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

    // 8. Overlapping-message feedback and reply anchoring (#6643/#6644): a
    // second DM arriving while a turn is still running gets an IMMEDIATE
    // busy notice quoting that second message, the working indicator and the
    // final reply quote the first message, and nothing is silently dropped
    // or left positionally ambiguous.
    const RACE_REPLY: &str = "anchored answer for the deferred-race leg";
    let race_first_body = dm_body(608, 717171, "what's the weather right now?");
    let (race_scope, _) = preresolve_vendor_turn_scope(
        &telegram_binding_service,
        &ironclaw_telegram_extension::TelegramChannelAdapter::default(),
        "telegram",
        TELEGRAM_INSTALLATION,
        &[(
            ironclaw_telegram_extension::TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            "itest_linked_bot".to_string(),
        )],
        &evidence,
        &race_first_body,
        false,
    )
    .await;
    let paused = Arc::new(PausedReplyGateway::new(RACE_REPLY));
    inbound.register_scope_gateway_for_test(
        race_scope.clone(),
        Arc::clone(&paused) as Arc<dyn HostManagedModelGateway>,
    );
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &race_first_body,
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let first_run = paused.wait_for_run_id().await;
    // The second message arrives while the first run is parked on its model
    // call — the genuine mid-run overlap from the issue report.
    let status = ingress
        .post(
            TELEGRAM_ROUTE,
            &dm_body(609, 717171, "and what about tomorrow?"),
            vec![(
                "X-Telegram-Bot-Api-Secret-Token",
                TELEGRAM_WEBHOOK_SECRET.to_string(),
            )],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let race_bodies = || -> Vec<serde_json::Value> {
        inbound
            .captured_network_requests_for_test()
            .iter()
            .filter(|request| request.url.ends_with("/sendMessage"))
            .filter_map(|request| serde_json::from_slice(&request.body).ok())
            .filter(|body: &serde_json::Value| body["chat_id"] == "717171")
            .collect()
    };
    let anchored_count = |bodies: &[serde_json::Value], needle: &str, anchor: i64| -> usize {
        bodies
            .iter()
            .filter(|body| {
                body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(needle))
                    && body["reply_to_message_id"] == anchor
            })
            .count()
    };
    // The busy notice must land while the first run is STILL parked on its
    // model call — immediacy is the #6643 contract (feedback arrives during
    // the run, not after it finishes). The paused gateway holds the first
    // run open, so this poll can only pass on admission-time feedback.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if anchored_count(&race_bodies(), "still working on a previous message", 619) == 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the overlapping DM must get its anchored busy notice while the \
             first run is still executing; saw: {:?}",
            race_bodies()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    paused.release();
    let coordinator = inbound.turn_coordinator_for_test();
    wait_for_run_status_in_scope(&coordinator, &race_scope, first_run, TurnStatus::Completed).await;
    ingress.drain().await;
    // Every race-chat sendMessage quotes the message it belongs to: exactly
    // one working indicator and one final reply anchored to the FIRST
    // message (dm_body assigns update_id + 10 → 618), the one busy notice
    // anchored to the SECOND (619), and no other anchors. Bounded poll —
    // the final reply lands observer-driven after drain returns.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let bodies = race_bodies();
        // The working indicator is the race-chat message anchored to 618 that is
        // not the final reply (its copy varies per run, so it can't be matched by
        // literal — content is pinned in the assistant's prompt unit test).
        let working_indicator_anchored_to_618 = bodies
            .iter()
            .filter(|body| {
                body["reply_to_message_id"].as_i64() == Some(618)
                    && !body["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(RACE_REPLY))
            })
            .count();
        if anchored_count(&bodies, RACE_REPLY, 618) == 1 && working_indicator_anchored_to_618 == 1 {
            assert_eq!(
                anchored_count(&bodies, "still working on a previous message", 619),
                1,
                "the busy notice stays a single anchored message: {bodies:?}"
            );
            assert!(
                bodies
                    .iter()
                    .all(|body| matches!(body["reply_to_message_id"].as_i64(), Some(618 | 619))),
                "every race-chat message must anchor to one of the two prompting \
                 messages: {bodies:?}"
            );
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the working indicator and final reply must each anchor to their \
             own prompt exactly once; saw: {bodies:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
