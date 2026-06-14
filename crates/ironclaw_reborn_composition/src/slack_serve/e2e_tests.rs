//! Minimal Slack Reborn E2E routing tests.
//!
//! These drive the real Slack route, native adapter runner, ProductWorkflow,
//! preconfigured actor binding, and final-reply observer with fake downstream
//! turn/outbound ports. They intentionally do not reuse the legacy Slack channel
//! or legacy pairing store.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use ironclaw_conversations::InMemoryConversationServices;
use ironclaw_host_api::{AgentId, ApprovalRequestId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_outbound::{
    CommunicationPreferenceRepository, InMemoryOutboundStateStore, OutboundStateStore,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, DeliveryStatus, EgressCredentialHandle, EgressRequest, EgressResponse,
    ExternalActorRef, OutboundDeliverySink, ProductAdapter, ProtocolHttpEgress,
    ProtocolHttpEgressError,
};
use ironclaw_product_workflow::{
    ApprovalInteractionActionView, ApprovalInteractionDecision, ApprovalInteractionScope,
    ApprovalInteractionService, AuthInteractionDecision, AuthInteractionService,
    DefaultInboundTurnService, DefaultProductWorkflow, InMemoryIdempotencyLedger,
    ListPendingApprovalsRequest, ListPendingApprovalsResponse, ListPendingAuthInteractionsRequest,
    ListPendingAuthInteractionsResponse, PendingApprovalInteractionView, ProductActorUserResolver,
    ProductConversationBindingService, ProductInstallationKey, ProductInstallationScope,
    ProductWorkflowError, ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
    ResolveAuthInteractionRequest, ResolveAuthInteractionResponse, StaticProductActorUserResolver,
    StaticProductInstallationResolver,
};
use ironclaw_slack_v2_adapter::{
    SLACK_USER_ACTOR_KIND, SlackV2Adapter, SlackV2AdapterConfig,
    slack_request_signature_auth_requirement,
};
use ironclaw_threads::{
    AppendAssistantDraftRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse, RunProfileId,
    RunProfileVersion, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator,
    TurnError, TurnId, TurnRunId, TurnRunState, TurnScope, TurnStatus,
};
use ironclaw_wasm_product_adapters::{
    HmacWebhookAuth, NativeProductAdapterRunner, NativeProductAdapterRunnerConfig, WebhookAuth,
};
use tower::ServiceExt;

use super::*;
use crate::slack_delivery::{
    SlackFinalReplyDeliveryObserver, SlackFinalReplyDeliveryServices,
    SlackFinalReplyDeliverySettings,
};
use crate::{
    AuthChallengeProvider, RebornUserIdentityLookup, RebornUserIdentityLookupError,
    SlackUserIdentityActorResolver,
};

#[path = "e2e_auth_challenge.rs"]
mod e2e_auth_challenge;
use e2e_auth_challenge::FakeAuthChallengeProvider;

const TENANT: &str = "tenant:slack";
const AGENT: &str = "agent:slack";
const PROJECT: &str = "project:slack";
const USER: &str = "user:slack-alice";
const ADAPTER: &str = "slack_v2";
const INSTALLATION: &str = "install_alpha";
const TEAM: &str = "T-A";
const SLACK_USER: &str = "U123";
const CHANNEL: &str = "D123";
const SLACK_SIGNATURE_HEADER: &str = "X-Slack-Signature";
const SLACK_TIMESTAMP_HEADER: &str = "X-Slack-Request-Timestamp";
const SECRET: &str = "topsecret";
const GATE: &str = "gate:approve-slack";
const GATE_B: &str = "gate:approve-slack-b";
const AUTH_GATE: &str = "gate:auth-slack";

struct Harness {
    mount: PublicRouteMount,
    state: SlackEventsRouteState,
    egress: RecordingEgress,
    coordinator: Arc<RecordingTurnCoordinator>,
    approvals: Arc<RecordingApprovalInteractionService>,
    auths: Arc<RecordingAuthInteractionService>,
    route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore>,
}

type HmacSha256 = Hmac<sha2::Sha256>;

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after Unix epoch") // safety: supported test platforms have post-epoch clocks.
        .as_secs()
}

fn slack_signature(timestamp: u64, body: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(SECRET.as_bytes()).expect("HMAC accepts any key size"); // safety: HMAC-SHA256 accepts arbitrary key lengths.
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body.as_bytes());
    format!("v0={:x}", mac.finalize().into_bytes())
}

impl Harness {
    async fn post_event(&self, body: &'static str) -> axum::response::Response {
        let timestamp = current_unix_timestamp();
        self.post_event_with_signature(body, timestamp, slack_signature(timestamp, body))
            .await
    }

    async fn post_retry_event(
        &self,
        body: &'static str,
        retry_num: u32,
    ) -> axum::response::Response {
        let timestamp = current_unix_timestamp();
        let signature = slack_signature(timestamp, body);
        self.mount
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(SLACK_EVENTS_PATH)
                    .header(SLACK_TIMESTAMP_HEADER, timestamp.to_string())
                    .header(SLACK_SIGNATURE_HEADER, signature)
                    .header("X-Slack-Retry-Num", retry_num.to_string())
                    .body(Body::from(body))
                    .expect("request should build"), // safety: static test request fixtures are valid.
            )
            .await
            .expect("router should respond") // safety: in-process test router should not fail
    }

    async fn post_event_with_signature(
        &self,
        body: &'static str,
        timestamp: u64,
        signature: String,
    ) -> axum::response::Response {
        self.mount
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(SLACK_EVENTS_PATH)
                    .header(SLACK_TIMESTAMP_HEADER, timestamp.to_string())
                    .header(SLACK_SIGNATURE_HEADER, signature)
                    .body(Body::from(body))
                    .expect("request should build"), // safety: static test request fixtures are valid.
            )
            .await
            .expect("router should respond") // safety: in-process test router should not fail
    }

    async fn drain(&self) {
        self.state.drain_immediate_ack_tasks().await;
    }

    fn slack_messages(&self) -> Vec<serde_json::Value> {
        self.egress
            .requests()
            .into_iter()
            .filter(|request| request.path().as_str() == "/api/chat.postMessage")
            .map(|request| {
                serde_json::from_slice(request.body()).expect("Slack JSON body") // safety: Slack adapter emits JSON request bodies in this test.
            })
            .collect()
    }

    fn slack_deletes(&self) -> Vec<serde_json::Value> {
        self.egress
            .requests()
            .into_iter()
            .filter(|request| request.path().as_str() == "/api/chat.delete")
            .map(|request| {
                serde_json::from_slice(request.body()).expect("Slack JSON body") // safety: Slack adapter emits JSON request bodies in this test.
            })
            .collect()
    }
}

async fn build_harness(mode: TurnMode) -> Harness {
    build_harness_with_actor_user_resolver(mode, static_personal_actor_user_resolver()).await
}

async fn build_harness_with_actor_user_resolver(
    mode: TurnMode,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
) -> Harness {
    build_harness_with_actor_user_resolver_and_auth_challenges(mode, actor_user_resolver, None)
        .await
}

async fn build_harness_with_actor_user_resolver_and_auth_challenges(
    mode: TurnMode,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
    auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
) -> Harness {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversations.clone();

    let adapter_id = ironclaw_product_adapters::ProductAdapterId::new(ADAPTER).expect("adapter id"); // safety: static test adapter id is valid.
    let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation id"); // safety: static test installation id is valid.
    let adapter: Arc<dyn ProductAdapter> = Arc::new(SlackV2Adapter::new(SlackV2AdapterConfig {
        adapter_id: adapter_id.clone(),
        installation_id: installation_id.clone(),
        egress_credential_handle: EgressCredentialHandle::new("slack_bot_token").expect("handle"), // safety: static test handle is valid.
        auth_requirement: slack_request_signature_auth_requirement(),
    }));

    let scope = ProductInstallationScope::with_default_scope(
        TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
        AgentId::new(AGENT).expect("agent"),    // safety: static test agent id is valid.
        Some(
            ProjectId::new(PROJECT).expect("project"), // safety: static test project id is valid.
        ),
    )
    .with_default_subject_user_id(UserId::new(USER).expect("user")) // safety: static test user id is valid.
    .with_actor_user_resolver(actor_user_resolver, actor_pairings);
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(adapter_id, installation_id.clone()),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, resolver);

    let threads = InMemorySessionThreadService::default();
    let coordinator = RecordingTurnCoordinator::new(threads.clone(), mode);
    let approvals = Arc::new(RecordingApprovalInteractionService::new(
        coordinator.clone(),
        threads.clone(),
    ));
    let auths = Arc::new(RecordingAuthInteractionService::new(coordinator.clone()));
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());

    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        threads.clone(),
        coordinator.clone(),
    ));
    let workflow = Arc::new(
        DefaultProductWorkflow::new(
            inbound,
            Arc::new(InMemoryIdempotencyLedger::new()),
            Arc::new(binding.clone()),
        )
        .with_approval_interaction_service(approvals.clone())
        .with_auth_interaction_service(auths.clone())
        .with_delivered_gate_routes(route_store.clone()),
    );

    let runner = Arc::new(NativeProductAdapterRunner::with_config(
        adapter.clone(),
        workflow,
        WebhookAuth::Hmac(HmacWebhookAuth::new(
            SLACK_SIGNATURE_HEADER,
            SLACK_TIMESTAMP_HEADER,
            SECRET.as_bytes().to_vec(),
            INSTALLATION,
        )),
        NativeProductAdapterRunnerConfig::new(
            Duration::from_secs(2),
            NonZeroUsize::new(4).expect("nonzero"), // safety: 4 is non-zero.
        ),
    ));

    let outbound = Arc::new(InMemoryOutboundStateStore::default());
    let outbound_store: Arc<dyn OutboundStateStore> = outbound.clone();
    let preferences: Arc<dyn CommunicationPreferenceRepository> = outbound;
    let egress = RecordingEgress::default();
    let sink = RecordingDeliverySink::default();
    let observer = Arc::new(SlackFinalReplyDeliveryObserver::with_settings(
        SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(binding),
            thread_service: Arc::new(threads),
            turn_coordinator: Arc::new(coordinator.clone()),
            outbound_store,
            route_store: route_store.clone(),
            communication_preferences: preferences,
            adapter,
            egress: Arc::new(egress.clone()),
            delivery_sink: Arc::new(sink),
            auth_challenges,
        },
        SlackFinalReplyDeliverySettings {
            poll_interval: Duration::from_millis(1),
            max_wait: Duration::from_secs(2),
            max_concurrent_deliveries: std::num::NonZeroUsize::new(4).expect("nonzero"), // safety: static test literal is non-zero.
            max_pending_deliveries: std::num::NonZeroUsize::new(16).expect("nonzero"), // safety: static test literal is non-zero.
        },
    ));

    let slack_resolver = StaticSlackInstallationResolver::new(vec![
        SlackInstallationRecord::new(
            TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            installation_id,
            SlackInstallationSelector::team(TEAM),
            runner,
        )
        .with_workflow_observer(observer),
    ]);
    let state = SlackEventsRouteState::from_resolver(Arc::new(slack_resolver));
    let mount = slack_events_route_mount(state.clone());

    Harness {
        mount,
        state,
        egress,
        coordinator: Arc::new(coordinator),
        approvals,
        auths,
        route_store,
    }
}

fn static_personal_actor_user_resolver() -> Arc<dyn ProductActorUserResolver> {
    Arc::new(StaticProductActorUserResolver::new([(
        ExternalActorRef::new(SLACK_USER_ACTOR_KIND, SLACK_USER, None::<String>).expect("actor"), // safety: static Slack actor ref is valid.
        UserId::new(USER).expect("user"), // safety: static test user id is valid.
    )]))
}

fn user_identity_actor_user_resolver() -> Arc<dyn ProductActorUserResolver> {
    Arc::new(SlackUserIdentityActorResolver::new(Arc::new(
        RecordingUserIdentityLookup::new([(
            format!("{INSTALLATION}:{SLACK_USER}"),
            UserId::new(USER).expect("user"), // safety: static test user id is valid.
        )]),
    )))
}

/// A scope-aware approval service used in delivered-gate-route E2E tests.
///
/// `list_pending` always returns an empty list, simulating the case where the
/// turn being approved lives on a foreign thread scope (not the inbound DM
/// scope). When `dispatch_scoped_approval_resolution` sees an empty pending
/// list it falls back to the delivered-gate-route conversation index.
/// `resolve` delegates to the inner recording service so request assertions
/// still work.
struct ForeignScopeApprovalService {
    inner: Arc<RecordingApprovalInteractionService>,
}

#[async_trait]
impl ApprovalInteractionService for ForeignScopeApprovalService {
    async fn list_pending(
        &self,
        _request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        Ok(ListPendingApprovalsResponse {
            approvals: Vec::new(),
        })
    }

    async fn resolve(
        &self,
        request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        self.inner.resolve(request).await
    }
}

/// Builds a harness where `list_pending` always returns empty, driving
/// `dispatch_scoped_approval_resolution` through the delivered-gate-route
/// fallback path. Returns the harness (with `route_store` accessible) and the
/// underlying recording approval service for request assertions.
///
/// By default, two separate `InMemoryDeliveredGateRouteStore` instances are used:
///
/// - `workflow_route_store` (exposed via `harness.route_store`): the store the
///   workflow queries when resolving delivered-gate-route fallback paths.  Tests
///   seed records here to control which routes the workflow sees.
/// - `observer_route_store`: the store the delivery observer writes to when it
///   auto-records a gate route after posting an approval prompt.  It is not
///   exposed because tests never need to read it — it intentionally stays
///   separate so auto-created routes cannot pollute the workflow's view and
///   accidentally turn a `Miss` into a `Single` or `Ambiguous`.
///
/// The unified-store regression harness below opts into sharing the same store
/// across observer and workflow to verify the production wiring shape.
async fn build_harness_for_delivered_route_tests()
-> (Harness, Arc<RecordingApprovalInteractionService>) {
    build_harness_for_delivered_route_tests_with_store_mode(false).await
}

async fn build_harness_for_unified_delivered_route_test()
-> (Harness, Arc<RecordingApprovalInteractionService>) {
    build_harness_for_delivered_route_tests_with_store_mode(true).await
}

async fn build_harness_for_delivered_route_tests_with_store_mode(
    share_observer_and_workflow_route_store: bool,
) -> (Harness, Arc<RecordingApprovalInteractionService>) {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversations.clone();

    let adapter_id = ironclaw_product_adapters::ProductAdapterId::new(ADAPTER).expect("adapter id"); // safety: static test adapter id is valid.
    let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation id"); // safety: static test installation id is valid.
    let adapter: Arc<dyn ProductAdapter> = Arc::new(SlackV2Adapter::new(SlackV2AdapterConfig {
        adapter_id: adapter_id.clone(),
        installation_id: installation_id.clone(),
        egress_credential_handle: EgressCredentialHandle::new("slack_bot_token").expect("handle"), // safety: static test handle is valid.
        auth_requirement: slack_request_signature_auth_requirement(),
    }));

    let scope = ProductInstallationScope::with_default_scope(
        TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
        AgentId::new(AGENT).expect("agent"),    // safety: static test agent id is valid.
        Some(
            ProjectId::new(PROJECT).expect("project"), // safety: static test project id is valid.
        ),
    )
    .with_default_subject_user_id(UserId::new(USER).expect("user")) // safety: static test user id is valid.
    .with_actor_user_resolver(static_personal_actor_user_resolver(), actor_pairings);
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(adapter_id, installation_id.clone()),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, resolver);

    let threads = InMemorySessionThreadService::default();
    let coordinator = RecordingTurnCoordinator::new(threads.clone(), TurnMode::BlockApproval);
    let inner_approvals = Arc::new(RecordingApprovalInteractionService::new(
        coordinator.clone(),
        threads.clone(),
    ));
    let foreign_approvals: Arc<dyn ApprovalInteractionService> =
        Arc::new(ForeignScopeApprovalService {
            inner: inner_approvals.clone(),
        });
    let auths = Arc::new(RecordingAuthInteractionService::new(coordinator.clone()));

    // workflow_route_store: queried by the workflow during delivered-route fallback.
    // Tests seed records here to control the outcome (Miss / Single / Ambiguous).
    let workflow_route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    // observer_route_store: written by the delivery observer when it auto-records a
    // gate route after posting an approval prompt.  Kept separate so auto-created
    // routes never bleed into the workflow's index.
    let observer_route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        if share_observer_and_workflow_route_store {
            workflow_route_store.clone()
        } else {
            Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default())
        };

    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        threads.clone(),
        coordinator.clone(),
    ));
    let workflow = Arc::new(
        DefaultProductWorkflow::new(
            inbound,
            Arc::new(InMemoryIdempotencyLedger::new()),
            Arc::new(binding.clone()),
        )
        .with_approval_interaction_service(foreign_approvals)
        .with_auth_interaction_service(auths.clone())
        .with_delivered_gate_routes(workflow_route_store.clone()),
    );

    let runner = Arc::new(NativeProductAdapterRunner::with_config(
        adapter.clone(),
        workflow,
        WebhookAuth::Hmac(HmacWebhookAuth::new(
            SLACK_SIGNATURE_HEADER,
            SLACK_TIMESTAMP_HEADER,
            SECRET.as_bytes().to_vec(),
            INSTALLATION,
        )),
        NativeProductAdapterRunnerConfig::new(
            Duration::from_secs(2),
            NonZeroUsize::new(4).expect("nonzero"), // safety: 4 is non-zero.
        ),
    ));

    let outbound = Arc::new(InMemoryOutboundStateStore::default());
    let outbound_store: Arc<dyn OutboundStateStore> = outbound.clone();
    let preferences: Arc<dyn CommunicationPreferenceRepository> = outbound;
    let egress = RecordingEgress::default();
    let sink = RecordingDeliverySink::default();
    let observer = Arc::new(SlackFinalReplyDeliveryObserver::with_settings(
        SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(binding),
            thread_service: Arc::new(threads),
            turn_coordinator: Arc::new(coordinator.clone()),
            outbound_store,
            route_store: observer_route_store,
            communication_preferences: preferences,
            adapter,
            egress: Arc::new(egress.clone()),
            delivery_sink: Arc::new(sink),
            auth_challenges: None,
        },
        SlackFinalReplyDeliverySettings {
            poll_interval: Duration::from_millis(1),
            max_wait: Duration::from_secs(2),
            max_concurrent_deliveries: std::num::NonZeroUsize::new(4).expect("nonzero"), // safety: static test literal is non-zero.
            max_pending_deliveries: std::num::NonZeroUsize::new(16).expect("nonzero"), // safety: static test literal is non-zero.
        },
    ));

    let slack_resolver = StaticSlackInstallationResolver::new(vec![
        SlackInstallationRecord::new(
            TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            installation_id,
            SlackInstallationSelector::team(TEAM),
            runner,
        )
        .with_workflow_observer(observer),
    ]);
    let state = SlackEventsRouteState::from_resolver(Arc::new(slack_resolver));
    let mount = slack_events_route_mount(state.clone());

    let harness = Harness {
        mount,
        state,
        egress,
        coordinator: Arc::new(coordinator),
        approvals: inner_approvals.clone(),
        auths,
        route_store: workflow_route_store,
    };
    (harness, inner_approvals)
}

/// Returns the conversation fingerprint for the DM channel used in the E2E
/// test fixtures: team_id="T-A", channel="D123", no thread_ts.
///
/// `length_prefixed_fingerprint(["T-A", "D123", ""])` = `"3:T-A|4:D123|0:|"`.
fn dm_conversation_fingerprint() -> String {
    ironclaw_conversations::ExternalConversationRef::new(Some(TEAM), CHANNEL, None, None)
        .expect("DM conversation ref") // safety: static test DM ref is valid.
        .conversation_fingerprint()
}

/// Returns a `TurnScope` representing a triggered run that lives on a thread
/// different from the DM binding thread — the "foreign scope" the approval
/// prompt was originally delivered for.
fn foreign_run_scope() -> TurnScope {
    TurnScope::new_with_owner(
        TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
        Some(AgentId::new(AGENT).expect("agent")), // safety: static test agent id is valid.
        Some(ProjectId::new(PROJECT).expect("project")), // safety: static test project id is valid.
        ThreadId::new("thread:foreign-triggered-run").expect("thread"), // safety: static test thread id is valid.
        Some(UserId::new(USER).expect("user")), // safety: static test user id is valid.
    )
}

// ── Delivered-gate-route approval E2E tests ───────────────────────────────────

/// Bare `approve` in the DM resolves the gate on the run's foreign scope via the
/// delivered-gate-route index.
///
/// Scenario: a triggered run is blocked on approval in a non-DM thread. The
/// approval prompt was delivered to the user's DM (recorded in the route store).
/// When the user replies with bare "approve" in the DM, `list_pending` on the DM
/// scope returns nothing (the run is on a different thread). The workflow falls
/// back to the conversation-fingerprint index, finds the route record, rewrites
/// the approval request to the run's original scope, and forwards it to the inner
/// approval service. The request recorded by the inner service must carry the
/// foreign scope and the correct run_id_hint.
#[tokio::test]
async fn bare_approve_in_dm_resolves_gate_on_foreign_scope_via_delivered_route() {
    let (harness, inner_approvals) = build_harness_for_delivered_route_tests().await;

    // Submit a turn so the DM conversation binding is created and the run is
    // tracked in the coordinator as blocked on approval.
    let block_response = harness.post_event(DM_BLOCK).await;
    assert_eq!(block_response.status(), StatusCode::OK);
    harness.drain().await;
    let blocked_run_id = harness
        .coordinator
        .blocked_run_id()
        .expect("run must be blocked after DM_BLOCK"); // safety: E2E test assertion.

    // Seed the route record: DM fingerprint → foreign scope, run_id = blocked run.
    harness
        .route_store
        .record_delivered_gate_route(ironclaw_outbound::DeliveredGateRouteRecord {
            tenant_id: TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            user_id: UserId::new(USER).expect("user"), // safety: static test user id is valid.
            gate_ref: GATE.to_string(),
            run_id: blocked_run_id,
            scope: foreign_run_scope(),
            recorded_at: chrono::Utc::now(),
            delivered_conversation_fingerprints: vec![dm_conversation_fingerprint()],
        })
        .await
        .expect("route record write"); // safety: in-memory store should not fail.

    // Post the bare approve. list_pending returns [] (ForeignScopeApprovalService),
    // so the workflow falls back to the conversation fingerprint index.
    let approve_response = harness.post_event(DM_APPROVE).await;
    assert_eq!(approve_response.status(), StatusCode::OK);
    harness.drain().await;

    let requests = inner_approvals.requests();
    assert_eq!(requests.len(), 1, "exactly one approval resolve request");
    assert_eq!(
        requests[0].scope.thread_id,
        foreign_run_scope().thread_id,
        "scope was rewritten to the foreign run's thread"
    );
    assert_eq!(
        requests[0].run_id_hint,
        Some(blocked_run_id),
        "run_id_hint carries the route record's run_id"
    );
    assert_eq!(
        requests[0].decision,
        ApprovalInteractionDecision::ApproveOnce
    );
}

#[tokio::test]
async fn bare_approve_in_dm_resolves_gate_recorded_by_observer() {
    let (harness, inner_approvals) = build_harness_for_unified_delivered_route_test().await;

    let block_response = harness.post_event(DM_BLOCK).await;
    assert_eq!(block_response.status(), StatusCode::OK);
    harness.drain().await;
    let blocked_run_id = harness
        .coordinator
        .blocked_run_id()
        .expect("run must be blocked after DM_BLOCK"); // safety: E2E test assertion.

    let approve_response = harness.post_event(DM_APPROVE).await;
    assert_eq!(approve_response.status(), StatusCode::OK);
    harness.drain().await;

    let requests = inner_approvals.requests();
    assert_eq!(requests.len(), 1, "exactly one approval resolve request");
    assert_eq!(
        requests[0].run_id_hint,
        Some(blocked_run_id),
        "run_id_hint must come from the observer-recorded route"
    );
    assert_eq!(
        requests[0].decision,
        ApprovalInteractionDecision::ApproveOnce
    );
}

/// Bare `approve gate:<ref>` (explicit gate ref) in the DM resolves through the
/// *direct* path (binding found, no delivered-route rewrite), even when a route
/// record for the DM is seeded.
///
/// When the DM binding already exists, `dispatch_approval_resolution` forwards
/// the request directly to the approval service using the DM scope. The
/// delivered-gate-route index is not consulted. The test documents this boundary:
/// explicit gate-ref does not produce a cross-scope rewrite.
#[tokio::test]
async fn explicit_gate_ref_approve_resolves_via_delivered_route() {
    let (harness, inner_approvals) = build_harness_for_delivered_route_tests().await;

    // Submit a turn to establish the DM binding and a blocked run.
    let block_response = harness.post_event(DM_BLOCK).await;
    assert_eq!(block_response.status(), StatusCode::OK);
    harness.drain().await;
    let blocked_run_id = harness
        .coordinator
        .blocked_run_id()
        .expect("run must be blocked after DM_BLOCK"); // safety: E2E test assertion.

    // Seed the route record (same as Test 1).
    harness
        .route_store
        .record_delivered_gate_route(ironclaw_outbound::DeliveredGateRouteRecord {
            tenant_id: TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            user_id: UserId::new(USER).expect("user"), // safety: static test user id is valid.
            gate_ref: GATE.to_string(),
            run_id: blocked_run_id,
            scope: foreign_run_scope(),
            recorded_at: chrono::Utc::now(),
            delivered_conversation_fingerprints: vec![dm_conversation_fingerprint()],
        })
        .await
        .expect("route record write"); // safety: in-memory store should not fail.

    // Post explicit gate ref.  The DM binding is found so dispatch_approval_resolution
    // forwards directly to the inner service without delivered-route rewrite.
    let approve_response = harness.post_event(DM_APPROVE_EXPLICIT_GATE).await;
    assert_eq!(approve_response.status(), StatusCode::OK);
    harness.drain().await;

    let requests = inner_approvals.requests();
    assert_eq!(requests.len(), 1, "exactly one approval resolve request");
    // Gate ref is carried correctly even on the direct path.
    assert_eq!(requests[0].gate_ref.as_str(), GATE);
    // run_id_hint is None on the direct path (no delivered-route record consulted).
    assert_eq!(
        requests[0].run_id_hint, None,
        "direct path does not carry run_id_hint"
    );
}

/// Bare `approve` in the DM with two live route records for the same conversation
/// is rejected as ambiguous — the workflow posts a "declined by policy" hint and
/// does NOT forward any resolve to the approval service.
#[tokio::test]
async fn bare_approve_with_two_live_routes_fails_closed_ambiguous() {
    let (harness, inner_approvals) = build_harness_for_delivered_route_tests().await;

    // Submit a turn to establish the DM binding (no blocked run needed for
    // this path — the route fallback fires when list_pending returns []).
    let block_response = harness.post_event(DM_BLOCK).await;
    assert_eq!(block_response.status(), StatusCode::OK);
    harness.drain().await;

    // Seed two route records, both delivered to the same DM, with different gate
    // refs — ambiguous.
    let fingerprint = dm_conversation_fingerprint();
    harness
        .route_store
        .record_delivered_gate_route(ironclaw_outbound::DeliveredGateRouteRecord {
            tenant_id: TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            user_id: UserId::new(USER).expect("user"), // safety: static test user id is valid.
            gate_ref: GATE.to_string(),
            run_id: ironclaw_turns::TurnRunId::new(),
            scope: foreign_run_scope(),
            recorded_at: chrono::Utc::now(),
            delivered_conversation_fingerprints: vec![fingerprint.clone()],
        })
        .await
        .expect("first route record write"); // safety: in-memory store should not fail.
    harness
        .route_store
        .record_delivered_gate_route(ironclaw_outbound::DeliveredGateRouteRecord {
            tenant_id: TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            user_id: UserId::new(USER).expect("user"), // safety: static test user id is valid.
            gate_ref: GATE_B.to_string(),
            run_id: ironclaw_turns::TurnRunId::new(),
            scope: foreign_run_scope(),
            recorded_at: chrono::Utc::now(),
            delivered_conversation_fingerprints: vec![fingerprint],
        })
        .await
        .expect("second route record write"); // safety: in-memory store should not fail.

    // Post bare approve with two ambiguous routes.
    let approve_response = harness.post_event(DM_APPROVE).await;
    assert_eq!(approve_response.status(), StatusCode::OK);
    harness.drain().await;

    // No resolve must have been forwarded to the approval service.
    assert!(
        inner_approvals.requests().is_empty(),
        "ambiguous routes must not reach the approval service"
    );

    // The user must receive an "ambiguous" hint (approve gate:<ref>) — not silence, not
    // a "declined by policy" message.  The approval prompt posted by the DM_BLOCK drain
    // occupies messages[0]; the rejection hint is messages[1].
    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        2,
        "expected approval prompt (DM_BLOCK) + ambiguous hint (DM_APPROVE), got {} message(s)",
        messages.len()
    );
    // AmbiguousResolution hint: "Multiple requests are pending … approve gate:<ref> …"
    // The hint uses the literal placeholder `<ref>`, which the approval prompt does not.
    assert!(
        messages[1]["text"]
            .as_str()
            .is_some_and(|t| t.contains("approve gate:<ref>")),
        "hint must prompt user to pick one gate ref; got: {:?}",
        messages[1]["text"]
    );
}

/// Bare `approve` in the DM with no delivered-route record reports a "couldn't
/// match" hint and does NOT forward any resolve to the approval service.
///
/// Scenario: the user sends a completed turn (binding is established, no gate is
/// blocked), then immediately replies `approve`.  `list_pending` returns an empty
/// list because no run is blocked, and no route record exists in the
/// conversation-fingerprint index (the approval prompt was never delivered to this
/// conversation).  The workflow falls back to the index, finds nothing, returns
/// `MissingGate`, and the delivery observer posts a `BindingRequired` hint.
///
/// This test uses a `TurnMode::Complete` harness instead of the
/// `ForeignScopeApprovalService` harness so that no approval prompt — and
/// therefore no auto-created route record — is ever posted to the DM.
#[tokio::test]
async fn bare_approve_with_no_route_still_reports_binding_hint() {
    let harness = build_harness(TurnMode::Complete {
        assistant_text: "done".into(),
    })
    .await;

    // Submit a completed turn to establish the DM binding.  No approval prompt is
    // delivered (TurnMode::Complete), so no delivered-gate-route record is created.
    let hello_response = harness.post_event(dm_message("Ev-final", "hello")).await;
    assert_eq!(hello_response.status(), StatusCode::OK);
    harness.drain().await;

    // Post bare approve.  list_pending returns [] (no run is blocked) and the
    // conversation-fingerprint index is empty → MissingGate → BindingRequired hint.
    let approve_response = harness.post_event(DM_APPROVE).await;
    assert_eq!(approve_response.status(), StatusCode::OK);
    harness.drain().await;

    // No resolve forwarded to the approval service (MissingGate path).
    assert!(
        harness.approvals.requests().is_empty(),
        "missing route must not reach the approval service"
    );

    // The user must receive a "couldn't match" hint.  The completed-turn reply
    // ("done") occupies messages[0]; the BindingRequired hint is messages[1].
    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        2,
        "expected final-reply (hello turn) + binding hint (DM_APPROVE), got {} message(s)",
        messages.len()
    );
    // BindingRequired hint: "I couldn't match this reply … use `approve gate:<ref>`."
    // This uses the literal placeholder `<ref>`.
    let hint_text = messages[1]["text"].as_str().unwrap_or("");
    assert!(
        hint_text.contains("approve gate:<ref>"),
        "hint must prompt user to use explicit gate ref; got: {hint_text:?}"
    );
}

#[tokio::test]
async fn slack_events_rejects_forged_hmac_signature() {
    let harness = build_harness(TurnMode::Complete {
        assistant_text: "must not send".into(),
    })
    .await;

    let response = harness
        .post_event_with_signature(
            dm_message("Ev-forged", "hello"),
            current_unix_timestamp(),
            "v0=deadbeef".to_string(),
        )
        .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    harness.drain().await;
    assert!(harness.slack_messages().is_empty());
}

#[tokio::test]
async fn slack_dm_delivers_final_reply_after_immediate_ack() {
    let harness = build_harness(TurnMode::Complete {
        assistant_text: "hello from reborn".into(),
    })
    .await;

    let response = harness.post_event(dm_message("Ev-final", "hello")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_body(response, "ok").await;
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], CHANNEL);
    assert_eq!(messages[0]["text"], "hello from reborn");
}

#[tokio::test]
async fn slack_dm_for_personally_bound_user_routes_through_reborn_identity() {
    let harness = build_harness_with_actor_user_resolver(
        TurnMode::Complete {
            assistant_text: "hello personal Slack binding".into(),
        },
        user_identity_actor_user_resolver(),
    )
    .await;

    let response = harness.post_event(dm_message("Ev-identity", "hello")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_body(response, "ok").await;
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], CHANNEL);
    assert_eq!(messages[0]["text"], "hello personal Slack binding");
}

#[tokio::test]
async fn slack_dm_retry_delivery_is_idempotent() {
    let harness = build_harness(TurnMode::Complete {
        assistant_text: "hello from reborn".into(),
    })
    .await;
    let body = dm_message("Ev-final", "hello");

    let first = harness.post_event(body).await;
    let retry = harness.post_retry_event(body, 1).await;

    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(retry.status(), StatusCode::OK);
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], CHANNEL);
    assert_eq!(messages[0]["text"], "hello from reborn");
}

#[tokio::test]
async fn slack_dm_delivers_approval_prompt_after_immediate_ack() {
    let harness = build_harness(TurnMode::BlockApproval).await;

    let response = harness
        .post_event(dm_message("Ev-approval", "needs approval"))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], CHANNEL);
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("Approval needed"))
    );
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("approve` or `deny"))
    );
    assert!(harness.slack_deletes().is_empty());
}

#[tokio::test]
async fn slack_dm_delivers_auth_prompt_with_setup_link_after_immediate_ack() {
    let auth_provider = Arc::new(FakeAuthChallengeProvider::default());
    let auth_challenges: Arc<dyn AuthChallengeProvider> = auth_provider.clone();
    let harness = build_harness_with_actor_user_resolver_and_auth_challenges(
        TurnMode::BlockAuth,
        static_personal_actor_user_resolver(),
        Some(auth_challenges),
    )
    .await;

    let response = harness
        .post_event(dm_message("Ev-auth", "needs auth"))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], CHANNEL);
    let text = messages[0]["text"].as_str().expect("Slack message text");
    assert!(text.contains("Authentication required"));
    assert!(text.contains("Setup link: https://provider.example/oauth"));
    assert!(harness.slack_deletes().is_empty());
    auth_provider.assert_single_call();
}

#[tokio::test]
async fn slack_channel_auth_prompt_omits_setup_link_after_immediate_ack() {
    let auth_challenges: Arc<dyn AuthChallengeProvider> =
        Arc::new(FakeAuthChallengeProvider::default());
    let harness = build_harness_with_actor_user_resolver_and_auth_challenges(
        TurnMode::BlockAuth,
        static_personal_actor_user_resolver(),
        Some(auth_challenges),
    )
    .await;

    let response = harness
        .post_event(app_mention_message("Ev-auth-channel", "needs auth"))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], "C123");
    assert_eq!(messages[0]["thread_ts"], "1710000000.000008");
    let text = messages[0]["text"].as_str().expect("Slack message text");
    assert!(text.contains("Authentication required"));
    assert!(!text.contains("Setup link:"));
    assert!(!text.contains("https://provider.example/oauth"));
    assert!(harness.slack_deletes().is_empty());
}

#[tokio::test]
async fn slack_dm_delivers_final_reply_after_auth_completes_outside_slack() {
    let auth_provider = Arc::new(FakeAuthChallengeProvider::default());
    let auth_challenges: Arc<dyn AuthChallengeProvider> = auth_provider.clone();
    let harness = build_harness_with_actor_user_resolver_and_auth_challenges(
        TurnMode::BlockAuth,
        static_personal_actor_user_resolver(),
        Some(auth_challenges),
    )
    .await;

    let response = harness
        .post_event(dm_message("Ev-auth", "needs auth"))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    for _ in 0..80 {
        if harness.slack_messages().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], CHANNEL);
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("Authentication required"))
    );

    harness
        .coordinator
        .resume_blocked_run_to_running()
        .await
        .expect("resume auth-blocked run");
    for _ in 0..80 {
        if harness.slack_messages().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["channel"], CHANNEL);
    assert_eq!(messages[1]["text"], "Ironclaw is thinking...");

    harness
        .coordinator
        .complete_active_run("authenticated and finished")
        .await
        .expect("complete resumed auth run");
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2]["channel"], CHANNEL);
    assert_eq!(messages[2]["text"], "authenticated and finished");
    let deletes = harness.slack_deletes();
    assert_eq!(deletes.len(), 2);
    assert_eq!(deletes[0]["channel"], CHANNEL);
    assert_eq!(deletes[1]["channel"], CHANNEL);
    auth_provider.assert_single_call();
}

#[tokio::test]
async fn slack_dm_posts_working_indicator_and_deletes_it_after_final_reply() {
    let harness = build_harness(TurnMode::Running).await;

    let response = harness.post_event(dm_message("Ev-working", "think")).await;

    assert_eq!(response.status(), StatusCode::OK);
    for _ in 0..80 {
        if harness.slack_messages().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], CHANNEL);
    assert_eq!(messages[0]["text"], "Ironclaw is thinking...");

    harness
        .coordinator
        .complete_active_run("done thinking")
        .await
        .expect("complete running turn");
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["channel"], CHANNEL);
    assert_eq!(messages[1]["text"], "done thinking");
    let deletes = harness.slack_deletes();
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0]["channel"], CHANNEL);
}

#[tokio::test]
async fn slack_approval_reply_resumes_and_delivers_final_reply() {
    let harness = build_harness(TurnMode::BlockApproval).await;

    let first = harness
        .post_event(dm_message("Ev-block", "needs approval"))
        .await;
    assert_eq!(first.status(), StatusCode::OK);
    harness.drain().await;
    assert_eq!(harness.slack_messages().len(), 1);

    let second = harness
        .post_event(dm_message("Ev-approve", "approve"))
        .await;

    assert_eq!(second.status(), StatusCode::OK);
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["channel"], CHANNEL);
    assert_eq!(messages[1]["text"], "approved and finished");
    let approvals = harness.approvals.requests();
    assert_eq!(approvals.len(), 1);
    assert_eq!(
        approvals[0].decision,
        ApprovalInteractionDecision::ApproveOnce
    );
    assert_eq!(approvals[0].gate_ref.as_str(), GATE);
}

#[tokio::test]
async fn slack_thread_auth_deny_with_bot_mention_cancels_auth_gate_without_agent_turn() {
    let harness = build_harness(TurnMode::BlockAuth).await;

    let first = harness
        .post_event(app_mention_message("Ev-auth-cancel-start", "needs auth"))
        .await;
    assert_eq!(first.status(), StatusCode::OK); // safety: Slack E2E route assertion.
    harness.drain().await;
    assert_eq!(harness.slack_messages().len(), 1); // safety: Slack E2E delivery assertion.

    let second = harness
        .post_event(thread_message_event(
            "Ev-auth-cancel",
            "<@UBOT> auth deny gate:auth-slack",
            "1710000000.000009",
        ))
        .await;

    assert_eq!(second.status(), StatusCode::OK); // safety: Slack E2E route assertion.
    harness.drain().await;

    let auths = harness.auths.requests();
    assert_eq!(auths.len(), 1); // safety: Slack E2E auth routing assertion.
    assert_eq!(auths[0].decision, AuthInteractionDecision::Deny); // safety: length asserted above.
    assert_eq!(auths[0].gate_ref.as_str(), AUTH_GATE); // safety: length asserted above.
    let submitted_turn_count = harness.coordinator.submitted_turn_count();
    assert_eq!(submitted_turn_count, 1); // safety: Slack E2E turn routing assertion.
    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 2); // safety: Slack E2E delivery assertion.
    assert_eq!(messages[1]["channel"], "C123");
    assert_eq!(messages[1]["thread_ts"], "1710000000.000009");
    assert_eq!(messages[1]["text"], "Authentication canceled.");
}

#[tokio::test]
async fn slack_dm_thread_auth_deny_cancels_base_dm_auth_gate_without_agent_turn() {
    let harness = build_harness(TurnMode::BlockAuth).await;

    let first = harness
        .post_event(dm_message("Ev-auth", "needs auth"))
        .await;
    assert_eq!(first.status(), StatusCode::OK); // safety: Slack E2E route assertion.
    harness.drain().await;
    assert_eq!(harness.slack_messages().len(), 1); // safety: Slack E2E delivery assertion.

    let second = harness
        .post_event(thread_message_event(
            "Ev-dm-auth-cancel",
            "`auth deny gate:auth-slack`",
            "1710000001.123456",
        ))
        .await;

    assert_eq!(second.status(), StatusCode::OK); // safety: Slack E2E route assertion.
    harness.drain().await;

    let auths = harness.auths.requests();
    assert_eq!(auths.len(), 1); // safety: Slack E2E auth routing assertion.
    assert_eq!(auths[0].decision, AuthInteractionDecision::Deny); // safety: length asserted above.
    assert_eq!(auths[0].gate_ref.as_str(), AUTH_GATE); // safety: length asserted above.
    let submitted_turn_count = harness.coordinator.submitted_turn_count();
    assert_eq!(submitted_turn_count, 1); // safety: Slack E2E turn routing assertion.
    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 2); // safety: Slack E2E delivery assertion.
    assert_eq!(messages[1]["channel"], CHANNEL);
    assert_eq!(messages[1]["thread_ts"], "1710000001.123456");
    assert_eq!(messages[1]["text"], "Authentication canceled.");
}

#[derive(Debug, Clone)]
enum TurnMode {
    Complete { assistant_text: String },
    Running,
    BlockApproval,
    BlockAuth,
}

#[derive(Clone)]
struct RecordingTurnCoordinator {
    state: Arc<Mutex<RecordingTurnState>>,
    threads: InMemorySessionThreadService,
    mode: TurnMode,
}

struct RecordingTurnState {
    runs: std::collections::HashMap<TurnRunId, TurnRunState>,
    active_run_id: Option<TurnRunId>,
    blocked_run_id: Option<TurnRunId>,
    submitted_turn_count: usize,
}

impl RecordingTurnCoordinator {
    fn new(threads: InMemorySessionThreadService, mode: TurnMode) -> Self {
        Self {
            state: Arc::new(Mutex::new(RecordingTurnState {
                runs: std::collections::HashMap::new(),
                active_run_id: None,
                blocked_run_id: None,
                submitted_turn_count: 0,
            })),
            threads,
            mode,
        }
    }

    fn blocked_run_id(&self) -> Option<TurnRunId> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .blocked_run_id
    }

    fn active_run_id(&self) -> Option<TurnRunId> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_run_id
    }

    fn submitted_turn_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .submitted_turn_count
    }

    async fn cancel_blocked_run(&self) -> Result<TurnRunId, ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let run_id =
            state
                .blocked_run_id
                .ok_or_else(|| ProductWorkflowError::TurnResumeRejected {
                    reason: "missing blocked run".into(),
                })?;
        let run = state.runs.get_mut(&run_id).ok_or_else(|| {
            ProductWorkflowError::TurnResumeRejected {
                reason: "missing blocked run state".into(),
            }
        })?;
        run.status = TurnStatus::Cancelled;
        run.gate_ref = None;
        state.blocked_run_id = None;
        Ok(run_id)
    }

    async fn complete_run(
        &self,
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        text: &str,
    ) -> Result<(), ProductWorkflowError> {
        append_final_assistant_message(&self.threads, &scope, run_id, text).await?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (reply_target_binding_ref, accepted_message_ref) = state
            .runs
            .get(&run_id)
            .map(|run| {
                (
                    run.reply_target_binding_ref.clone(),
                    run.accepted_message_ref.clone(),
                )
            })
            .unwrap_or_else(|| {
                (
                    ReplyTargetBindingRef::new("slack:reply-target").expect("reply target"), // safety: static test reply target is valid.
                    AcceptedMessageRef::new("slack:approval-reply").expect("accepted ref"), // safety: static test accepted ref is valid.
                )
            });
        state.runs.insert(
            run_id,
            turn_state(
                scope,
                actor,
                run_id,
                TurnStatus::Completed,
                None,
                reply_target_binding_ref,
                accepted_message_ref,
            ),
        );
        Ok(())
    }

    async fn resume_blocked_run_to_running(&self) -> Result<(), ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let run_id =
            state
                .blocked_run_id
                .ok_or_else(|| ProductWorkflowError::TurnResumeRejected {
                    reason: "missing blocked run".into(),
                })?;
        let run = state.runs.get_mut(&run_id).ok_or_else(|| {
            ProductWorkflowError::TurnResumeRejected {
                reason: "missing blocked run state".into(),
            }
        })?;
        run.status = TurnStatus::Running;
        run.gate_ref = None;
        state.active_run_id = Some(run_id);
        state.blocked_run_id = None;
        Ok(())
    }

    async fn complete_active_run(&self, text: &str) -> Result<(), ProductWorkflowError> {
        let run_id =
            self.active_run_id()
                .ok_or_else(|| ProductWorkflowError::TurnResumeRejected {
                    reason: "missing active run".into(),
                })?;
        self.complete_existing_run(run_id, text).await
    }

    async fn complete_existing_run(
        &self,
        run_id: TurnRunId,
        text: &str,
    ) -> Result<(), ProductWorkflowError> {
        let (scope, actor) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let run = state.runs.get(&run_id).ok_or_else(|| {
                ProductWorkflowError::TurnResumeRejected {
                    reason: "missing run state".into(),
                }
            })?;
            let actor =
                run.actor
                    .clone()
                    .ok_or_else(|| ProductWorkflowError::TurnResumeRejected {
                        reason: "missing run actor".into(),
                    })?;
            (run.scope.clone(), actor)
        };
        self.complete_run(scope, actor, run_id, text).await
    }
}

#[async_trait]
impl TurnCoordinator for RecordingTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let run_id = request.requested_run_id.unwrap_or_default();
        let status = match &self.mode {
            TurnMode::Complete { assistant_text } => {
                append_final_assistant_message(
                    &self.threads,
                    &request.scope,
                    run_id,
                    assistant_text,
                )
                .await
                .map_err(|error| TurnError::Unavailable {
                    reason: error.to_string(),
                })?;
                TurnStatus::Completed
            }
            TurnMode::Running => TurnStatus::Running,
            TurnMode::BlockApproval => TurnStatus::BlockedApproval,
            TurnMode::BlockAuth => TurnStatus::BlockedAuth,
        };
        let gate_ref = match status {
            TurnStatus::BlockedApproval => {
                Some(GateRef::new(GATE).expect("gate ref")) // safety: static test gate ref is valid.
            }
            TurnStatus::BlockedAuth => {
                Some(GateRef::new(AUTH_GATE).expect("auth gate ref")) // safety: static test gate ref is valid.
            }
            _ => None,
        };
        let response = SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id,
            status,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor::default(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
        };
        let run_state = turn_state(
            request.scope,
            request.actor,
            run_id,
            status,
            gate_ref,
            request.reply_target_binding_ref,
            request.accepted_message_ref,
        );
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.submitted_turn_count += 1;
        state.active_run_id = Some(run_id);
        if matches!(
            status,
            TurnStatus::BlockedApproval | TurnStatus::BlockedAuth
        ) {
            state.blocked_run_id = Some(run_id);
        }
        state.runs.insert(run_id, run_state);
        Ok(response)
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("approval test uses fake ApprovalInteractionService")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("cancel_run is not used")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .runs
            .get(&request.run_id)
            .cloned()
            .ok_or_else(|| TurnError::Unavailable {
                reason: "missing fake run state".into(),
            })
    }
}

async fn append_final_assistant_message(
    threads: &InMemorySessionThreadService,
    scope: &TurnScope,
    run_id: TurnRunId,
    text: &str,
) -> Result<(), ProductWorkflowError> {
    let thread_scope = ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id: scope
            .agent_id
            .clone()
            .ok_or_else(|| ProductWorkflowError::Transient {
                reason: "missing agent id in fake turn scope".into(),
            })?,
        project_id: scope.project_id.clone(),
        owner_user_id: Some(UserId::new(USER).expect("user")), // safety: static test user id is valid.
        mission_id: None,
    };
    let message = threads
        .append_assistant_draft(AppendAssistantDraftRequest {
            scope: thread_scope.clone(),
            thread_id: scope.thread_id.clone(),
            turn_run_id: run_id.to_string(),
            content: MessageContent::text(text),
        })
        .await
        .map_err(|error| ProductWorkflowError::Transient {
            reason: error.to_string(),
        })?;
    threads
        .finalize_assistant_message(
            &thread_scope,
            &scope.thread_id,
            message.message_id,
            MessageContent::text(text),
        )
        .await
        .map_err(|error| ProductWorkflowError::Transient {
            reason: error.to_string(),
        })?;
    Ok(())
}

fn turn_state(
    scope: TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
    status: TurnStatus,
    gate_ref: Option<GateRef>,
    reply_target_binding_ref: ReplyTargetBindingRef,
    accepted_message_ref: AcceptedMessageRef,
) -> TurnRunState {
    TurnRunState {
        scope,
        actor: Some(actor),
        turn_id: TurnId::new(),
        run_id,
        status,
        accepted_message_ref,
        source_binding_ref: ironclaw_turns::SourceBindingRef::new("slack:source")
            .expect("source binding"), // safety: static test source binding is valid.
        reply_target_binding_ref,
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor::default(),
    }
}

struct RecordingApprovalInteractionService {
    coordinator: RecordingTurnCoordinator,
    threads: InMemorySessionThreadService,
    requests: Mutex<Vec<ResolveApprovalInteractionRequest>>,
}

impl RecordingApprovalInteractionService {
    fn new(coordinator: RecordingTurnCoordinator, threads: InMemorySessionThreadService) -> Self {
        Self {
            coordinator,
            threads,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ResolveApprovalInteractionRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl ApprovalInteractionService for RecordingApprovalInteractionService {
    async fn list_pending(
        &self,
        request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        let Some(run_id) = self.coordinator.blocked_run_id() else {
            return Ok(ListPendingApprovalsResponse {
                approvals: Vec::new(),
            });
        };
        Ok(ListPendingApprovalsResponse {
            approvals: vec![PendingApprovalInteractionView {
                scope: ApprovalInteractionScope::from_turn(&request.scope, &request.actor),
                run_id,
                gate_ref: GateRef::new(GATE).map_err(|err| {
                    ProductWorkflowError::TurnSubmissionRejected {
                        reason: err.to_string(),
                    }
                })?,
                approval_request_id: ApprovalRequestId::new(),
                summary: "Approval needed".into(),
                action: ApprovalInteractionActionView::Other,
            }],
        })
    }

    async fn resolve(
        &self,
        request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request.clone());
        let run_id = self.coordinator.blocked_run_id().ok_or_else(|| {
            ProductWorkflowError::TurnResumeRejected {
                reason: "missing blocked run".into(),
            }
        })?;
        self.coordinator
            .complete_run(
                request.scope.clone(),
                request.actor.clone(),
                run_id,
                "approved and finished",
            )
            .await?;
        let _ = &self.threads;
        Ok(ResolveApprovalInteractionResponse::Approved(
            ResumeTurnResponse {
                run_id,
                status: TurnStatus::Completed,
                event_cursor: EventCursor::default(),
            },
        ))
    }
}

struct RecordingAuthInteractionService {
    coordinator: RecordingTurnCoordinator,
    requests: Mutex<Vec<ResolveAuthInteractionRequest>>,
}

impl RecordingAuthInteractionService {
    fn new(coordinator: RecordingTurnCoordinator) -> Self {
        Self {
            coordinator,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ResolveAuthInteractionRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl AuthInteractionService for RecordingAuthInteractionService {
    async fn list_pending(
        &self,
        _request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ProductWorkflowError> {
        Ok(ListPendingAuthInteractionsResponse {
            auth_interactions: Vec::new(),
        })
    }

    async fn resolve(
        &self,
        request: ResolveAuthInteractionRequest,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request.clone());
        let run_id = self.coordinator.cancel_blocked_run().await?;
        Ok(match request.decision {
            AuthInteractionDecision::Deny => {
                ResolveAuthInteractionResponse::Canceled(CancelRunResponse {
                    run_id,
                    status: TurnStatus::Cancelled,
                    event_cursor: EventCursor::default(),
                    already_terminal: false,
                    actor: None,
                })
            }
            AuthInteractionDecision::CredentialProvided { .. }
            | AuthInteractionDecision::CallbackCompleted { .. } => {
                ResolveAuthInteractionResponse::Resumed(ResumeTurnResponse {
                    run_id,
                    status: TurnStatus::Queued,
                    event_cursor: EventCursor::default(),
                })
            }
        })
    }
}

#[derive(Clone, Default)]
struct RecordingEgress {
    requests: Arc<Mutex<Vec<EgressRequest>>>,
}

impl RecordingEgress {
    fn requests(&self) -> Vec<EgressRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl ProtocolHttpEgress for RecordingEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        let response = slack_response_for_request(&request);
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        Ok(response)
    }
}

fn slack_response_for_request(request: &EgressRequest) -> EgressResponse {
    if request.path().as_str().starts_with("/api/chat.") {
        let has_json_content_type = request
            .headers()
            .iter()
            .any(|header| header.name() == "content-type" && header.value() == "application/json");
        if !has_json_content_type {
            return EgressResponse::new(
                200,
                br#"{"ok":false,"error":"missing_post_type"}"#.to_vec(),
            );
        }
    }
    if request.path().as_str() == "/api/chat.postMessage" {
        let body: serde_json::Value = match serde_json::from_slice(request.body()) {
            Ok(body) => body,
            Err(_) => {
                return EgressResponse::new(
                    200,
                    br#"{"ok":false,"error":"invalid_json"}"#.to_vec(),
                );
            }
        };
        let channel = body["channel"].as_str().unwrap_or("DTEST");
        let ts_seed = stable_slack_test_ts(request.body());
        return EgressResponse::new(
            200,
            serde_json::json!({
                "ok": true,
                "channel": channel,
                "ts": ts_seed,
            })
            .to_string()
            .into_bytes(),
        );
    }
    EgressResponse::new(200, br#"{"ok":true}"#.to_vec())
}

fn stable_slack_test_ts(body: &[u8]) -> String {
    let mut hash = 0_u64;
    for byte in body {
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(*byte));
    }
    format!("1710000001.{:06}", hash % 1_000_000)
}

#[derive(Default)]
struct RecordingDeliverySink {
    statuses: Mutex<Vec<DeliveryStatus>>,
}

#[async_trait]
impl OutboundDeliverySink for RecordingDeliverySink {
    async fn record(&self, status: DeliveryStatus) {
        self.statuses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(status);
    }
}

#[derive(Debug, Default)]
struct RecordingUserIdentityLookup {
    bindings: std::collections::HashMap<String, UserId>,
}

impl RecordingUserIdentityLookup {
    fn new(bindings: impl IntoIterator<Item = (String, UserId)>) -> Self {
        Self {
            bindings: bindings.into_iter().collect(),
        }
    }
}

#[async_trait]
impl RebornUserIdentityLookup for RecordingUserIdentityLookup {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        if provider != "slack" {
            return Ok(None);
        }
        Ok(self.bindings.get(provider_user_id).cloned())
    }
}

fn dm_message(event_id: &'static str, text: &'static str) -> &'static str {
    match (event_id, text) {
        ("Ev-final", "hello") => DM_FINAL,
        ("Ev-approval", "needs approval") => DM_APPROVAL,
        ("Ev-block", "needs approval") => DM_BLOCK,
        ("Ev-approve", "approve") => DM_APPROVE,
        ("Ev-approve-explicit", "approve gate:approve-slack") => DM_APPROVE_EXPLICIT_GATE,
        ("Ev-forged", "hello") => DM_FORGED,
        ("Ev-identity", "hello") => DM_IDENTITY,
        ("Ev-auth", "needs auth") => DM_AUTH,
        ("Ev-working", "think") => DM_WORKING,
        _ => panic!("unknown fixture"),
    }
}

fn app_mention_message(event_id: &'static str, text: &'static str) -> &'static str {
    match (event_id, text) {
        ("Ev-auth-channel", "needs auth") => APP_MENTION_AUTH,
        ("Ev-auth-cancel-start", "needs auth") => APP_MENTION_AUTH_CANCEL_START,
        _ => panic!("unknown fixture"),
    }
}

fn thread_message_event(
    event_id: &'static str,
    text: &'static str,
    thread_ts: &'static str,
) -> &'static str {
    match (event_id, text, thread_ts) {
        ("Ev-auth-cancel", "<@UBOT> auth deny gate:auth-slack", "1710000000.000009") => {
            THREAD_AUTH_CANCEL_WITH_MENTION
        }
        ("Ev-dm-auth-cancel", "`auth deny gate:auth-slack`", "1710000001.123456") => {
            DM_THREAD_AUTH_CANCEL
        }
        _ => panic!("unknown fixture"),
    }
}

async fn assert_body(response: axum::response::Response, expected: &str) {
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body collect") // safety: in-memory response body should collect in tests
        .to_bytes();
    assert_eq!(&body[..], expected.as_bytes()); // safety: assertion is inside the Slack E2E test helper.
}

const DM_FINAL: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-final",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"hello","ts":"1710000000.000001"}
	}"#;

const DM_APPROVAL: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-approval",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"needs approval","ts":"1710000000.000002"}
	}"#;

const DM_BLOCK: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-block",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"needs approval","ts":"1710000000.000003"}
	}"#;

const DM_APPROVE: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-approve",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"approve","ts":"1710000000.000004"}
	}"#;

const DM_FORGED: &str = r#"{
	  "type":"event_callback",
	  "team_id":"T-A",
	  "api_app_id":"A-slack",
	  "event_id":"Ev-forged",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"hello","ts":"1710000000.000005"}
	}"#;

const DM_IDENTITY: &str = r#"{
	  "type":"event_callback",
	  "team_id":"T-A",
	  "api_app_id":"A-slack",
	  "event_id":"Ev-identity",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"hello","ts":"1710000000.000006"}
	}"#;

const DM_AUTH: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-auth",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"needs auth","ts":"1710000000.000007"}
	}"#;

const DM_WORKING: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-working",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"think","ts":"1710000000.000009"}
	}"#;

const APP_MENTION_AUTH: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-auth-channel",
  "event":{"type":"app_mention","user":"U123","channel":"C123","text":"<@UBOT> needs auth","ts":"1710000000.000008"}
}"#;

const APP_MENTION_AUTH_CANCEL_START: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-auth-cancel-start",
  "event":{"type":"app_mention","user":"U123","channel":"C123","text":"<@UBOT> needs auth","ts":"1710000000.000009"}
}"#;

const THREAD_AUTH_CANCEL_WITH_MENTION: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-auth-cancel",
  "event":{"type":"message","user":"U123","channel":"C123","text":"<@UBOT> auth deny gate:auth-slack","ts":"1710000000.000010","thread_ts":"1710000000.000009"}
}"#;

const DM_THREAD_AUTH_CANCEL: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-dm-auth-cancel",
  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"`auth deny gate:auth-slack`","ts":"1710000001.123457","thread_ts":"1710000001.123456"}
}"#;

/// Explicit gate-ref approve in the DM: `approve gate:approve-slack`.
/// The gate ref token after "approve " is `gate:approve-slack` (= GATE).
/// Used by the delivered-gate-route test that verifies explicit gate ref resolves
/// directly (binding found → no cross-scope rewrite).
const DM_APPROVE_EXPLICIT_GATE: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-approve-explicit",
  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"approve gate:approve-slack","ts":"1710000000.000005"}
}"#;
