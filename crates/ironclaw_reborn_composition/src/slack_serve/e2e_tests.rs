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
use ironclaw_host_api::{AgentId, ApprovalRequestId, ProjectId, TenantId, UserId};
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
    ApprovalInteractionService, DefaultInboundTurnService, DefaultProductWorkflow,
    InMemoryIdempotencyLedger, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
    PendingApprovalInteractionView, ProductActorUserResolver, ProductConversationBindingService,
    ProductInstallationKey, ProductInstallationScope, ProductWorkflowError,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
    StaticProductActorUserResolver, StaticProductInstallationResolver,
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
const AUTH_GATE: &str = "gate:auth-slack";

struct Harness {
    mount: PublicRouteMount,
    state: SlackEventsRouteState,
    egress: RecordingEgress,
    coordinator: Arc<RecordingTurnCoordinator>,
    approvals: Arc<RecordingApprovalInteractionService>,
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
        .with_approval_interaction_service(approvals.clone()),
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
        .complete_blocked_run("authenticated and finished")
        .await
        .expect("complete auth-blocked run");
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["channel"], CHANNEL);
    assert_eq!(messages[1]["text"], "authenticated and finished");
    auth_provider.assert_single_call();
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

#[derive(Debug, Clone)]
enum TurnMode {
    Complete { assistant_text: String },
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
    blocked_run_id: Option<TurnRunId>,
}

impl RecordingTurnCoordinator {
    fn new(threads: InMemorySessionThreadService, mode: TurnMode) -> Self {
        Self {
            state: Arc::new(Mutex::new(RecordingTurnState {
                runs: std::collections::HashMap::new(),
                blocked_run_id: None,
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

    async fn complete_blocked_run(&self, text: &str) -> Result<(), ProductWorkflowError> {
        let (scope, actor, run_id) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let run_id =
                state
                    .blocked_run_id
                    .ok_or_else(|| ProductWorkflowError::TurnResumeRejected {
                        reason: "missing blocked run".into(),
                    })?;
            let run = state.runs.get(&run_id).ok_or_else(|| {
                ProductWorkflowError::TurnResumeRejected {
                    reason: "missing blocked run state".into(),
                }
            })?;
            let actor =
                run.actor
                    .clone()
                    .ok_or_else(|| ProductWorkflowError::TurnResumeRejected {
                        reason: "missing blocked run actor".into(),
                    })?;
            (run.scope.clone(), actor, run_id)
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
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        Ok(EgressResponse::new(200, br#"{"ok":true}"#.to_vec()))
    }
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
        ("Ev-forged", "hello") => DM_FORGED,
        ("Ev-identity", "hello") => DM_IDENTITY,
        ("Ev-auth", "needs auth") => DM_AUTH,
        _ => panic!("unknown fixture"),
    }
}

fn app_mention_message(event_id: &'static str, text: &'static str) -> &'static str {
    match (event_id, text) {
        ("Ev-auth-channel", "needs auth") => APP_MENTION_AUTH,
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

const APP_MENTION_AUTH: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-auth-channel",
  "event":{"type":"app_mention","user":"U123","channel":"C123","text":"<@UBOT> needs auth","ts":"1710000000.000008"}
}"#;
