//! Slack-fixture E2E tests of the GENERIC channel host assembly (P6 S6.4).
//!
//! Ported from the retired `slack_serve/e2e_tests.rs` suite: the 24
//! behavioral scenarios (signed event -> turn -> coordinated reply, gate
//! routing, OAuth-identity actor resolution, triggered delivery, ...) now
//! drive the PRODUCTION assembly path — a real `ExtensionHost` activation of
//! the bundled Slack manifest, administrator configuration through
//! the manifest-indexed admin resolver, `GenericChannelHostAssembly` building the inbound
//! graph (durable workflow state, provider-identity actor resolution,
//! run-delivery observer), and the canonical generic-ingress route mount the
//! fixtures post to. Scripted turn/approval/auth/egress fakes fill
//! exactly the seams the production factory fills.
//!
//! One production-shape delta from the retired suite: the assembly wires the
//! SAME delivered-gate-route store into the workflow and the observer (the
//! retired harness could split them), so observer-recorded routes are always
//! visible to the workflow's fallback resolution — tests seed records over
//! the observer's auto-recorded ones where a scenario needs a specific
//! route.

// arch-exempt: large_file, the ported gate-route e2e coverage stays one
// suite; decomposition tracked in
// docs/plans/2026-07-02-reborn-internal-module-refactor.md.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use http_body_util::BodyExt;
use ironclaw_extension_host::{
    AdminConfigurationIdempotencyKey, AdminConfigurationService, AdminConfigurationStore,
    AdminConfigurationSubmittedValue,
};
use ironclaw_extensions::{
    ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionInstallationStorePort, ExtensionManifestRecord, ExtensionManifestRef,
    InstallationOwner, ManifestSource,
};
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, ApprovalRequestId, ExtensionId, InvocationId, ProjectId, ResourceScope, SecretHandle,
    TenantId, ThreadId, UserId,
};
use ironclaw_outbound::test_support::in_memory_backed_outbound_state_store;
use ironclaw_outbound::{
    CommunicationPreferenceRecord, CommunicationPreferenceRepository, DeliveredGateRouteStore,
    DeliveryDefaultScope, OutboundStateStorePort, WriteCommunicationPreferenceRequest,
};
use ironclaw_product::{
    AdapterInstallationId, AuthRequirement, AuthResolutionPayload, AuthResolutionResult,
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ParsedProductInbound,
    ProductAdapterId, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductTriggerReason, ProtocolAuthEvidence, TrustedInboundContext, UserMessagePayload,
};
use ironclaw_product::{
    ApprovalInteractionActionView, ApprovalInteractionDecision, ApprovalInteractionScope,
    ApprovalInteractionService, AuthInteractionDecision, AuthInteractionService,
    ConversationBindingService, CurrentDeliveryTarget, CurrentDeliveryTargetResolver,
    DeliveryCoordinator, DeliveryRetryPolicy, ListPendingApprovalsRequest,
    ListPendingApprovalsResponse, ListPendingAuthInteractionsRequest,
    ListPendingAuthInteractionsResponse, NoReplyContext, PendingApprovalInteractionView,
    PreferenceTargetCodec, ProductWorkflowError, ResolveApprovalInteractionRequest,
    ResolveApprovalInteractionResponse, ResolveAuthInteractionRequest,
    ResolveAuthInteractionResponse, ResolveBindingRequest, ResolvedBinding, RunDeliveryEventRouter,
    RunDeliveryServices, TriggeredRunDeliveryDriver, TriggeredRunDeliveryRequest,
};
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStorePort};
use ironclaw_slack_extension::{
    SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID, SlackPreferenceTargetCodec,
};
use ironclaw_telegram_extension::TelegramPreferenceTargetCodec;
use ironclaw_threads::{
    AppendAssistantDraftRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_triggers::{TriggerFire, TriggerFireIdentity, TriggerId};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, ProductTurnContext, ReplyTargetBindingRef, ResumeTurnRequest,
    ResumeTurnResponse, RunOriginAdapter, RunProfileId, RunProfileVersion, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnEventKind, TurnEventSink,
    TurnId, TurnLifecycleEvent, TurnOriginKind, TurnOwner, TurnRunId, TurnRunState, TurnScope,
    TurnStatus, TurnSurfaceType,
};
use tower::ServiceExt;

use ironclaw_extension_host::ExtensionHost;
use ironclaw_extension_host::egress::{ApprovedChannelEgress, ChannelEgressTransport};

use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

use super::{
    ChannelExtras, ChannelHostDeliveryDeps, ChannelHostIdentity, GenericChannelHostAssembly,
    GenericChannelHostDeps,
};

struct SlackTriggeredTargetResolver;

#[async_trait]
impl CurrentDeliveryTargetResolver for SlackTriggeredTargetResolver {
    async fn resolve_current_target(
        &self,
        _scope: &TurnScope,
        _actor: &TurnActor,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<CurrentDeliveryTarget>, ProductWorkflowError> {
        let codec = SlackPreferenceTargetCodec;
        let external_conversation_ref = codec
            .conversation_for_target(target)
            .ok_or(ProductWorkflowError::BindingAccessDenied)?;
        Ok(Some(CurrentDeliveryTarget {
            extension_id: "slack".to_string(),
            external_conversation_ref,
            personal_direct_message: codec.is_personal_direct_message(target),
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
use crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver;
use crate::extension_host::channel_delivery::{
    IngressReplyContextSource, SnapshotChannelDeliveryResolver,
};
use crate::extension_host::extension_ingress::{
    ExtensionIngressParts, PostAdmissionObserver, build_extension_ingress,
    extension_ingress_route_mount,
};
use crate::extension_host::run_delivery_ports::ProductAuthBlockedAuthPromptSource;
use crate::{RebornUserIdentityLookup, RebornUserIdentityLookupError};
use ironclaw_host_ingress::PublicRouteMount;
use ironclaw_product::AuthChallengeProvider;
use ironclaw_product::BlockedAuthPromptSource;

#[path = "e2e_auth_challenge.rs"]
mod e2e_auth_challenge;
use e2e_auth_challenge::FakeAuthChallengeProvider;

const TENANT: &str = "tenant:slack";
const AGENT: &str = "agent:slack";
const PROJECT: &str = "project:slack";
const USER: &str = "user:slack-alice";
/// The generic assembly keys the inbound graph by EXTENSION id.
const ADAPTER: &str = "slack";
const INSTALLATION: &str = "install_alpha";
const TELEGRAM_INSTALLATION: &str = "telegram_install_alpha";
const TEAM: &str = "T-A";
const SLACK_USER: &str = "U123";
const CHANNEL: &str = "D123";
const TELEGRAM_USER: &str = "424242";
const SLACK_SIGNATURE_HEADER: &str = "X-Slack-Signature";
const SLACK_TIMESTAMP_HEADER: &str = "X-Slack-Request-Timestamp";
const SECRET: &str = "topsecret";
const GATE: &str = "gate:approval-00000000-0000-0000-0000-000000000001";
const GATE_B: &str = "gate:approval-00000000-0000-0000-0000-000000000002";
const AUTH_GATE: &str = "gate:auth-slack";

fn outbound_reply_target(
    entry: &crate::outbound::OutboundDeliveryTargetEntry,
) -> ReplyTargetBindingRef {
    match &entry.destination {
        ironclaw_outbound::RunFinalReplyDestination::External {
            reply_target_binding_ref,
        } => reply_target_binding_ref.clone(),
        ironclaw_outbound::RunFinalReplyDestination::WebApp => {
            panic!("channel target must carry an external reply binding")
        }
    }
}

fn current_target_resolver(
    assembly: &Arc<GenericChannelHostAssembly>,
    registry: Arc<crate::outbound::MutableOutboundDeliveryTargetRegistry>,
) -> Arc<dyn CurrentDeliveryTargetResolver> {
    let resolver = Arc::new(
        crate::extension_host::channel_outbound_targets::ComposedCurrentDeliveryTargetResolver::new(
            registry,
        ),
    );
    resolver
        .attach_assembly(assembly)
        .expect("attach current delivery target resolver");
    resolver
}

fn slack_manifest_from_bundled_inventory() -> String {
    ironclaw_first_party_extensions::packages::bundled_packages()
        .into_iter()
        .find(|bundle| bundle.id == "slack")
        .expect("Slack is in the bundled package inventory") // safety: Slack is a compile-time bundled test fixture.
        .manifest_toml
        .into_owned()
}

fn telegram_manifest_from_bundled_inventory() -> String {
    ironclaw_first_party_extensions::packages::bundled_packages()
        .into_iter()
        .find(|bundle| bundle.id == "telegram")
        .expect("Telegram is in the bundled package inventory") // safety: Telegram is a compile-time bundled test fixture.
        .manifest_toml
        .into_owned()
}

/// The canonical generic-ingress path the fixtures post to: the single
/// `extension_ingress_route_mount` serves
/// `/webhooks/extensions/{extension_id}/{route_suffix}` for every active
/// channel extension.
const SLACK_EVENTS_PATH: &str = "/webhooks/extensions/slack/events";

struct Harness {
    mount: PublicRouteMount,
    /// The generic ingress registry: `drain()` settles every route-owned
    /// in-flight task (the assembly registered the sink's drain with it).
    ingress: ExtensionIngressParts,
    egress: RecordingEgress,
    coordinator: Arc<RecordingTurnCoordinator>,
    approvals: Arc<RecordingApprovalInteractionService>,
    auths: Arc<RecordingAuthInteractionService>,
    route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore>,
    identity_lookup: Arc<RecordingUserIdentityLookup>,
    /// The production configure service backing the assembly — admission
    /// scenarios save routing values through it mid-test.
    admin_configuration_resolver: Arc<ComposedExtensionAdminConfigurationResolver>,
    /// Durable caller-membership authority used by the production outbound
    /// target provider. The active host snapshot remains deployment-global.
    installation_store: Arc<ExtensionInstallationStore>,
    /// The harness's outbound state store — the SAME allocation the
    /// assembly's delivery deps read communication preferences from, so
    /// tests can seed the creator's personal preference.
    outbound: Arc<ironclaw_outbound::OutboundStateStore<ironclaw_filesystem::InMemoryBackend>>,
    /// Keeps the harness extension host (and its published snapshot) alive.
    _host: Arc<ExtensionHost>,
    /// Keeps the assembly (and its reconcile loop + registrations) alive.
    assembly: Arc<GenericChannelHostAssembly>,
    event_router: Arc<RunDeliveryEventRouter>,
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
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
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
        self.ingress.registry.drain().await;
    }

    /// Ensure a foreign-scope thread exists in the harness thread service.
    /// The scripted coordinator's `complete_run` appends the final message to
    /// the resolved scope's thread; with the P4 sync-admission transport that
    /// append happens before the webhook response, so a missing scripted
    /// thread surfaces as a 5xx instead of hiding behind the old
    /// immediate-ack 200.
    async fn ensure_scope_thread(&self, scope: &TurnScope) {
        self.coordinator
            .threads
            .ensure_thread(EnsureThreadRequest {
                scope: ThreadScope {
                    tenant_id: scope.tenant_id.clone(),
                    agent_id: scope
                        .agent_id
                        .clone()
                        .unwrap_or_else(|| AgentId::new(AGENT).expect("agent")), // safety: static test agent id is valid.
                    project_id: scope.project_id.clone(),
                    owner_user_id: scope.thread_owner.explicit_owner_user_id().cloned(),
                    mission_id: None,
                },
                thread_id: Some(scope.thread_id.clone()),
                created_by_actor_id: "test-actor".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure scripted foreign thread"); // safety: in-memory test thread service should not fail.
    }

    fn slack_messages(&self) -> Vec<serde_json::Value> {
        self.egress.bodies_for("/api/chat.postMessage")
    }

    fn slack_deletes(&self) -> Vec<serde_json::Value> {
        self.egress.bodies_for("/api/chat.delete")
    }

    fn telegram_messages(&self) -> Vec<serde_json::Value> {
        self.egress.bodies_for("/sendMessage")
    }
}

/// Options every harness variant composes; the core builder is the single
/// place the production assembly is stood up.
struct HarnessOptions {
    mode: TurnMode,
    auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
    /// Activate the bundled Telegram extension and register its production
    /// preference codec for the cross-provider outbound-target regression.
    telegram: bool,
    /// Wrap the recording approval service in [`ForeignScopeApprovalService`]
    /// (empty `list_pending`) so bare gate replies exercise the
    /// delivered-gate-route fallback.
    foreign_scope_approvals: bool,
}

impl HarnessOptions {
    fn new(mode: TurnMode) -> Self {
        Self {
            mode,
            auth_challenges: None,
            telegram: false,
            foreign_scope_approvals: false,
        }
    }
}

async fn build_harness(mode: TurnMode) -> Harness {
    build_harness_with_options(HarnessOptions::new(mode)).await
}

async fn build_harness_with_telegram(mode: TurnMode) -> Harness {
    let mut options = HarnessOptions::new(mode);
    options.telegram = true;
    build_harness_with_options(options).await
}

async fn build_harness_with_auth_challenges(
    mode: TurnMode,
    auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
) -> Harness {
    let mut options = HarnessOptions::new(mode);
    options.auth_challenges = auth_challenges;
    build_harness_with_options(options).await
}

/// Harness for the delivered-gate-route scenarios: `list_pending` always
/// returns empty (the blocked run lives on a foreign thread scope), driving
/// `dispatch_scoped_approval_resolution` through the conversation-fingerprint
/// route index. Returns the inner recording approval service for request
/// assertions.
async fn build_harness_for_delivered_route_tests()
-> (Harness, Arc<RecordingApprovalInteractionService>) {
    let mut options = HarnessOptions::new(TurnMode::BlockApproval);
    options.foreign_scope_approvals = true;
    let harness = build_harness_with_options(options).await;
    let approvals = Arc::clone(&harness.approvals);
    (harness, approvals)
}

/// The production wiring shape: with the generic assembly the observer and
/// the workflow ALWAYS share one delivered-gate-route store, so the
/// "unified" scenario is simply the delivered-route harness.
async fn build_harness_for_unified_delivered_route_test()
-> (Harness, Arc<RecordingApprovalInteractionService>) {
    build_harness_for_delivered_route_tests().await
}

/// The core builder: real host + real manifest + administrator saves +
/// the production `GenericChannelHostAssembly`, with scripted downstream
/// fakes at exactly the seams the production factory fills.
async fn build_harness_with_options(options: HarnessOptions) -> Harness {
    let threads = InMemorySessionThreadService::default();
    let run_delivery_events = Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test());
    let coordinator = RecordingTurnCoordinator::new(
        threads.clone(),
        options.mode.clone(),
        Arc::clone(&run_delivery_events),
    );
    let approvals = Arc::new(RecordingApprovalInteractionService::new(
        coordinator.clone(),
        threads.clone(),
    ));
    let auths = Arc::new(RecordingAuthInteractionService::new(coordinator.clone()));
    let approval_interaction: Arc<dyn ApprovalInteractionService> =
        if options.foreign_scope_approvals {
            Arc::new(ForeignScopeApprovalService {
                inner: approvals.clone(),
            })
        } else {
            approvals.clone()
        };
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let outbound =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let outbound_store: Arc<dyn OutboundStateStorePort> = outbound.clone();
    let preferences: Arc<dyn CommunicationPreferenceRepository> = outbound.clone();
    let egress = RecordingEgress::default();

    let host = channel_test_extension_host(options.telegram).await;
    let installation_store = channel_test_installation_store(options.telegram).await;
    let ingress = build_extension_ingress(
        host.snapshot_watch(),
        Arc::new(ironclaw_extension_host::DeploymentChannelRegistry::default()),
        Arc::new(
            crate::extension_host::reply_contexts::ReplyContextStore::new(
                Arc::new(InMemoryBackend::new()),
                TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
                UserId::new(USER).expect("user"),       // safety: static test user id is valid.
            ),
        ),
    );
    let delivery_coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&outbound_store),
        Arc::new(SnapshotChannelDeliveryResolver::new(
            host.snapshot_watch(),
            Arc::new(egress.clone()),
        )),
        Arc::new(IngressReplyContextSource::new(Arc::clone(
            &ingress.reply_context,
        ))),
        DeliveryRetryPolicy {
            max_attempts: 2,
            backoff: Duration::ZERO,
        },
    ));

    let identity_lookup = Arc::new(RecordingUserIdentityLookup::new([
        (
            format!("{INSTALLATION}:{SLACK_USER}"),
            UserId::new(USER).expect("user"), // safety: static test user id is valid.
        ),
        (
            format!("{TELEGRAM_INSTALLATION}:{TELEGRAM_USER}"),
            UserId::new(USER).expect("user"), // safety: static test user id is valid.
        ),
    ]));
    let outbound_delivery_targets =
        Arc::new(crate::outbound::MutableOutboundDeliveryTargetRegistry::default());
    let current_delivery_targets = Arc::new(
        crate::extension_host::channel_outbound_targets::ComposedCurrentDeliveryTargetResolver::new(
            Arc::clone(&outbound_delivery_targets),
        ),
    );

    let admin_configuration_resolver = configured_admin_configuration_resolver().await;
    let deps = GenericChannelHostDeps {
        watch: host.snapshot_watch(),
        deployment_channels: Arc::new(ironclaw_extension_host::DeploymentChannelRegistry::default()),
        registry: Arc::clone(&ingress.registry),
        admin_configuration_resolver: Arc::clone(&admin_configuration_resolver),
        workflow_state: Arc::new(ironclaw_product::ChannelWorkflowStateService::new(
            Arc::new(InMemoryBackend::new()),
        )),
        thread_service: Arc::new(threads.clone()),
        turn_coordinator: Arc::new(coordinator.clone()),
        approval_interaction: Some(approval_interaction),
        auth_interaction: Some(auths.clone() as Arc<dyn AuthInteractionService>),
        identity: ChannelHostIdentity {
            tenant_id: TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            agent_id: AgentId::new(AGENT).expect("agent"), // safety: static test agent id is valid.
            project_id: Some(ProjectId::new(PROJECT).expect("project")), // safety: static test project id is valid.
            operator_user_id: UserId::new(USER).expect("user"), // safety: static test user id is valid.
        },
        identity_lookup: Some(Arc::clone(&identity_lookup)
            as Arc<dyn crate::provider_identity::RebornUserIdentityLookup>),
        delivery: Some(ChannelHostDeliveryDeps {
            coordinator: delivery_coordinator,
            outbound_store,
            route_store: Arc::clone(&route_store),
            communication_preferences: preferences,
            current_delivery_targets: Arc::clone(&current_delivery_targets)
                as Arc<dyn CurrentDeliveryTargetResolver>,
            approval_context: None,
            blocked_auth_prompts: options.auth_challenges.map(|provider| {
                Arc::new(ProductAuthBlockedAuthPromptSource::new(Some(provider)))
                    as Arc<dyn BlockedAuthPromptSource>
            }),
            auth_flow_cancel: None,
            event_router: Arc::clone(&run_delivery_events),
        }),
        channel_pairing: None,
    };
    let assembly = GenericChannelHostAssembly::start(deps);
    current_delivery_targets
        .attach_assembly(&assembly)
        .expect("attach current delivery target resolver");
    // Vendor extras exactly as the binary's channel-extension binding feeds
    // them: the preference-target codec, with gate replies owned generically
    // by the shared sink.
    assembly
        .register_extras(
            "slack",
            ChannelExtras {
                preference_target_codec: Some(Arc::new(SlackPreferenceTargetCodec)),
                subject_route_resolver: None,
            },
        )
        .await;
    if options.telegram {
        assembly
            .register_extras(
                "telegram",
                ChannelExtras {
                    preference_target_codec: Some(Arc::new(TelegramPreferenceTargetCodec)),
                    subject_route_resolver: None,
                },
            )
            .await;
    }

    let mount =
        extension_ingress_route_mount(&ingress).expect("extension ingress route mount builds"); // safety: bundled manifest projects a valid ingress descriptor.

    Harness {
        mount,
        ingress,
        egress,
        coordinator: Arc::new(coordinator),
        approvals,
        auths,
        route_store,
        identity_lookup,
        admin_configuration_resolver,
        installation_store,
        outbound,
        _host: host,
        assembly,
        event_router: run_delivery_events,
    }
}

async fn channel_test_installation_store(
    include_telegram: bool,
) -> Arc<ExtensionInstallationStore> {
    let store = Arc::new(crate::extension_host::filesystem_installation_store_for_test().await);
    let members = InstallationOwner::users(BTreeSet::from([
        UserId::new(USER).expect("Alice user"),
        UserId::new("user:slack-bob").expect("Bob user"),
    ]))
    .expect("non-empty fixture members");
    for (manifest_toml, installation_id) in
        [(slack_manifest_from_bundled_inventory(), INSTALLATION)]
            .into_iter()
            .chain(include_telegram.then(|| {
                (
                    telegram_manifest_from_bundled_inventory(),
                    TELEGRAM_INSTALLATION,
                )
            }))
    {
        let record = ExtensionManifestRecord::from_toml(
            manifest_toml,
            ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog().expect("host ports"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("bundled channel manifest resolves");
        let extension_id = record.resolved().id.clone();
        store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new(installation_id).expect("installation id"),
                    extension_id.clone(),
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    chrono::Utc::now(),
                    members.clone(),
                )
                .expect("channel installation"),
            )
            .await
            .expect("persist channel installation");
    }
    store
}

/// Production-shaped administrator configuration for the real Slack manifest.
async fn configured_admin_configuration_resolver()
-> Arc<ComposedExtensionAdminConfigurationResolver> {
    let record = ExtensionManifestRecord::from_toml(
        slack_manifest_from_bundled_inventory(),
        ManifestSource::HostBundled,
        &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"), // safety: default catalog is valid in tests.
        None,
        &product_extension_host_api_contract_registry().expect("contracts"), // safety: default registry is valid in tests.
    )
    .expect("bundled channel manifest resolves"); // safety: compile-time bundled manifest is valid.
    let manifest = Arc::new(record.resolved().clone());
    let scope = ResourceScope {
        tenant_id: TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
        user_id: UserId::new(USER).expect("user"),         // safety: static test user id is valid.
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    let secrets: Arc<dyn SecretStorePort> = Arc::new(SecretStore::ephemeral());
    let admin = Arc::new(
        AdminConfigurationService::new(
            AdminConfigurationStore::new(Arc::new(ScopedFilesystem::new(
                filesystem,
                crate::invocation_mount_view,
            ))),
            secrets,
            manifest.admin_configuration.clone(),
        )
        .expect("admin configuration service"), // safety: test-only fixture construction.
    );
    let group = manifest.admin_configuration[0].group_id.clone();
    let submitted = [
        ("slack_bot_token", "xoxb-test"),
        ("slack_signing_secret", SECRET),
        ("slack_team_id", TEAM),
        ("slack_api_app_id", "A-TEST"),
        ("slack_installation_id", INSTALLATION),
        ("slack_bot_user_id", "U-BOT"),
        ("slack_oauth_client_id", "oauth-client"),
        ("slack_oauth_client_secret", "oauth-secret"),
        ("slack_allowed_channels", r#"["C123"]"#),
    ]
    .into_iter()
    .map(|(handle, value)| AdminConfigurationSubmittedValue {
        handle: SecretHandle::new(handle).expect("secret handle"), // safety: static test handle is valid.
        value: SecretMaterial::from(value.to_string()),
    })
    .collect();
    admin
        .replace(
            &scope,
            &group,
            &AdminConfigurationIdempotencyKey::new("slack-channel-e2e-config")
                .expect("idempotency key"), // safety: static test key is valid.
            0,
            submitted,
        )
        .await
        .expect("save administrator configuration"); // safety: test fixture setup must succeed.
    Arc::new(ComposedExtensionAdminConfigurationResolver::new(
        admin,
        scope,
        [manifest],
    ))
}

/// The P4 generic-ingress transport: a minimal `ExtensionHost` with the real
/// bundled Slack manifest active and, for the Telegram regression, the
/// bundled Telegram manifest too, with their real channel adapters bound.
/// The snapshot is shared by the ingress router, generic outbound-target
/// provider, and delivery resolver.
async fn channel_test_extension_host(
    include_telegram: bool,
) -> Arc<ironclaw_extension_host::ExtensionHost> {
    use ironclaw_extension_host::test_support::{FakeToolAdapter, RecordingDrain};
    use ironclaw_extension_host::{
        BindContext, BindError, ExtensionBindings, ExtensionEntrypoint, ExtensionHost,
        ExtensionHostDeps, ExtensionLoader, InstallationRecord, InstallationState, LoadContext,
        LoadedExtension, RehydratedInstallationRecordStore,
    };

    struct ChannelTestLifecycleEgressFactory;
    impl ironclaw_extension_host::lifecycle::EgressFactory for ChannelTestLifecycleEgressFactory {
        fn egress_for_channel(
            &self,
            extension_id: &str,
            _installation_id: &str,
            _declared: &[ironclaw_host_api::ChannelEgressDescriptor],
        ) -> Arc<dyn ironclaw_host_api::RestrictedEgress> {
            Arc::new(ChannelTestLifecycleEgress {
                allow: extension_id == "telegram",
            })
        }
    }
    struct ChannelTestLifecycleEgress {
        allow: bool,
    }
    #[async_trait]
    impl ironclaw_host_api::RestrictedEgress for ChannelTestLifecycleEgress {
        async fn send(
            &self,
            _request: ironclaw_host_api::RestrictedEgressRequest,
        ) -> Result<
            ironclaw_host_api::RestrictedEgressResponse,
            ironclaw_host_api::RestrictedEgressError,
        > {
            if !self.allow {
                return Err(ironclaw_host_api::RestrictedEgressError::PolicyDenied);
            }
            Ok(ironclaw_host_api::RestrictedEgressResponse {
                status: 200,
                body: br#"{"ok":true}"#.to_vec(),
            })
        }
    }

    struct SlackTestEntrypoint;
    impl ExtensionEntrypoint for SlackTestEntrypoint {
        fn bind(&self, _ctx: BindContext) -> Result<ExtensionBindings, BindError> {
            Ok(ExtensionBindings {
                tools: Some(Arc::new(FakeToolAdapter)),
                channel: Some(Arc::new(ironclaw_slack_extension::SlackChannelAdapter)),
            })
        }
    }
    struct TelegramTestEntrypoint;
    impl ExtensionEntrypoint for TelegramTestEntrypoint {
        fn bind(&self, _ctx: BindContext) -> Result<ExtensionBindings, BindError> {
            Ok(ExtensionBindings {
                tools: None,
                channel: Some(Arc::new(
                    ironclaw_telegram_extension::TelegramChannelAdapter::default(),
                )),
            })
        }
    }
    struct ChannelTestLoader;
    #[async_trait]
    impl ExtensionLoader for ChannelTestLoader {
        async fn load(&self, ctx: &LoadContext) -> Result<LoadedExtension, BindError> {
            match ctx.extension_id.as_str() {
                "slack" => Ok(LoadedExtension::new(Box::new(SlackTestEntrypoint))),
                "telegram" => Ok(LoadedExtension::new(Box::new(TelegramTestEntrypoint))),
                extension_id => Err(BindError::Load {
                    reason: format!("unsupported channel fixture `{extension_id}`"),
                }),
            }
        }
    }

    let resolve_manifest = |manifest_toml: String| {
        let host_ports = ironclaw_host_runtime::default_host_port_catalog().expect("host ports"); // safety: default catalog is valid in tests.
        let contracts = product_extension_host_api_contract_registry().expect("contracts"); // safety: default registry is valid in tests.
        ironclaw_extensions::ExtensionManifestRecord::from_toml(
            manifest_toml,
            ironclaw_extensions::ManifestSource::HostBundled,
            &host_ports,
            None,
            &contracts,
        )
        .expect("bundled channel manifest resolves") // safety: compile-time bundled manifest is valid.
        .resolved()
        .clone()
    };
    let slack_resolved = resolve_manifest(slack_manifest_from_bundled_inventory());
    let host = Arc::new(
        ExtensionHost::new(ExtensionHostDeps {
            store: Arc::new(RehydratedInstallationRecordStore::default()),
            loader: Arc::new(ChannelTestLoader),
            drain: Arc::new(RecordingDrain::default()),
            egress: Arc::new(ChannelTestLifecycleEgressFactory),
            reserved_capability_ids: Default::default(),
            reserved_ingress_routes: Default::default(),
            hook_deadline: Duration::from_secs(5),
        })
        .await,
    );
    host.install(InstallationRecord {
        extension_id: "slack".to_string(),
        installation_id: INSTALLATION.to_string(),
        state: InstallationState::Installed,
        resolved: Arc::new(slack_resolved),
        config: Vec::new(),
        last_error: None,
    })
    .await
    .expect("install"); // safety: in-memory test host install should not fail.
    host.activate("slack").await.expect("activate"); // safety: scripted test loader binds valid adapters.
    if include_telegram {
        let telegram_resolved = resolve_manifest(telegram_manifest_from_bundled_inventory());
        host.install(InstallationRecord {
            extension_id: "telegram".to_string(),
            installation_id: TELEGRAM_INSTALLATION.to_string(),
            state: InstallationState::Installed,
            resolved: Arc::new(telegram_resolved),
            config: vec![(
                ironclaw_telegram_extension::TELEGRAM_WEBHOOK_URL_CONFIG.to_string(),
                "https://host.example/webhooks/extensions/telegram/updates".to_string(),
            )],
            last_error: None,
        })
        .await
        .expect("install Telegram"); // safety: in-memory test host install should not fail.
        host.activate("telegram").await.expect("activate Telegram"); // safety: scripted test loader binds valid adapters.
    }
    host
}

fn test_fallback_notice_scope() -> TurnScope {
    TurnScope::new_with_owner(
        TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
        Some(AgentId::new(AGENT).expect("agent")), // safety: static test agent id is valid.
        Some(ProjectId::new(PROJECT).expect("project")), // safety: static test project id is valid.
        ThreadId::new("slack-channel-notices").expect("thread"), // safety: static literal is valid.
        Some(UserId::new(USER).expect("user")), // safety: static test user id is valid.
    )
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
    harness.ensure_scope_thread(&foreign_run_scope()).await;
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

/// No-op [`ConversationBindingService`] mirroring the one the production
/// triggered-delivery factory (`build_triggered_run_delivery_hook_from_parts`)
/// hardcodes: the triggered path receives the `TurnScope` directly from the
/// trigger worker and never resolves a live inbound binding. Using it here
/// keeps the composite on the same seam the production assembly fills.
struct NoopTriggeredBindingService;

#[async_trait]
impl ConversationBindingService for NoopTriggeredBindingService {
    async fn resolve_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "NoopTriggeredBindingService is not used in triggered delivery".to_string(),
        })
    }

    async fn lookup_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "NoopTriggeredBindingService is not used in triggered delivery".to_string(),
        })
    }
}

/// State-backed [`TurnCoordinator`] for driving a [`TriggeredRunDeliveryDriver`].
///
/// The driver registers one lifecycle consumer and re-reads canonical state for
/// every committed event. This fake therefore keeps the supplied state stable;
/// it must never advance merely because a reader looked at it. For the
/// OAuth-not-DM backstop it also records `cancel_run`.
struct ScriptedTriggerCoordinator {
    state: Mutex<TurnRunState>,
    cancel_calls: Mutex<Vec<TurnRunId>>,
}

impl ScriptedTriggerCoordinator {
    fn new(template: TurnRunState) -> Self {
        Self {
            state: Mutex::new(template),
            cancel_calls: Mutex::new(Vec::new()),
        }
    }

    /// Number of `cancel_run` calls observed so far. Used by the OAuth-not-DM
    /// backstop test to assert the blocked run is cancelled exactly once.
    fn cancel_call_count(&self) -> usize {
        self.cancel_calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

#[async_trait]
impl TurnCoordinator for ScriptedTriggerCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        unreachable!("triggered delivery driver never prepares turns")
    }

    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("triggered delivery driver never submits turns")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("triggered delivery driver never resumes turns")
    }

    async fn retry_turn(
        &self,
        _request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        unreachable!("triggered delivery driver never retries turns")
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        // Reached only by the OAuth-not-DM backstop (`cancel_auth_blocked_run`),
        // which cancels the run before posting the auth-unavailable notice. The
        // approval-only scenario (`Self::new`) never triggers this arm.
        self.cancel_calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request.run_id);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.status = TurnStatus::Cancelled;
        state.gate_ref = None;
        Ok(CancelRunResponse {
            run_id: request.run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor::default(),
            already_terminal: false,
            actor: None,
        })
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }
}

/// Build a Slack personal-DM reply-target binding ref for team `T-A` /
/// channel `D123`, so the triggered run's approval prompt is delivered to the
/// same DM the inbound `approve` arrives on. The `space` segment (`T-A`) is what
/// the driver captures as `resolved_space_id`; combined with the posted channel
/// (`D123`, echoed by `RecordingEgress`) it yields `dm_conversation_fingerprint()`.
fn dm_reply_target_binding_ref() -> ReplyTargetBindingRef {
    fn seg(name: &str, value: &str) -> String {
        format!("{}:{}:{};", name, value.len(), value)
    }
    let raw = format!(
        "{}{}{}{}{}{}{}{}{}",
        seg("adapter", SLACK_V2_ADAPTER_ID),
        seg("installation", INSTALLATION),
        seg("agent", AGENT),
        seg("project", ""),
        seg("space", TEAM),
        seg("conversation", CHANNEL),
        seg("topic", ""),
        seg("actor_kind", SLACK_USER_ACTOR_KIND),
        seg("actor", SLACK_USER),
    );
    ironclaw_slack_extension::slack_reply_target_binding_ref_from_raw(raw)
        .expect("DM reply target binding ref") // safety: static test binding ref is valid.
}

/// Poll the shared delivered-gate-route store until the driver records a
/// route for `(tenant, user, gate_ref)` matching `matches`, then return it.
/// Times out after 5 s. The predicate matters under the production-unified
/// store: the inbound observer auto-records a route for the same gate ref
/// when it posts the DM approval prompt, so waits for the DRIVER's record
/// must match on its distinguishing scope rather than mere existence.
async fn wait_for_gate_route_matching(
    route_store: &dyn DeliveredGateRouteStore,
    tenant: &TenantId,
    user: &UserId,
    gate_ref: &str,
    matches: impl Fn(&ironclaw_outbound::DeliveredGateRouteRecord) -> bool,
) -> ironclaw_outbound::DeliveredGateRouteRecord {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let loaded = route_store
                .load_delivered_gate_route(tenant, user, gate_ref)
                .await
                .expect("load gate route"); // safety: test-only poll loop
            if let Some(record) = loaded
                && matches(&record)
            {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("driver records the delivered gate route within 5 s") // safety: test-only timeout; panic message is the failure diagnostic
}

/// Poll `egress`'s recorded requests until at least one Slack `chat.postMessage`
/// matching `predicate` has been captured, then return every such match. Times
/// out after 5 s with a panic message naming `description`.
///
/// Shared bounded-poll scaffold for `wait_for_approval_prompt_messages` and
/// `wait_for_auth_prompt_messages` below, and the "any posted message" wait
/// used by the OAuth-not-DM backstop test. The router dispatches lifecycle
/// delivery asynchronously, so the bounded wait observes the provider seam
/// without adding sleeps or timing assumptions to the production path.
/// Delivery fixture for triggered-driver tests: an independent slack
/// extension host + recording transport + coordinator (the driver posts are
/// isolated from the inbound harness's transport).
struct TriggeredDeliveryFixture {
    driver_egress: RecordingEgress,
    delivery_coordinator: Arc<DeliveryCoordinator>,
    _host: Arc<ironclaw_extension_host::ExtensionHost>,
}

async fn triggered_delivery_fixture(
    outbound_store: Arc<dyn OutboundStateStorePort>,
) -> TriggeredDeliveryFixture {
    let host = channel_test_extension_host(false).await;
    let driver_egress = RecordingEgress::default();
    let delivery_coordinator = Arc::new(DeliveryCoordinator::new(
        outbound_store,
        Arc::new(SnapshotChannelDeliveryResolver::new(
            host.snapshot_watch(),
            Arc::new(driver_egress.clone()),
        )),
        Arc::new(NoReplyContext),
        DeliveryRetryPolicy {
            max_attempts: 2,
            backoff: Duration::ZERO,
        },
    ));
    TriggeredDeliveryFixture {
        driver_egress,
        delivery_coordinator,
        _host: host,
    }
}

/// Translate a trigger fire into the generic driver's request — the same
/// mapping the production Slack post-submit hook performs.
fn triggered_request_from_fire(
    fire: &TriggerFire,
    run_id: TurnRunId,
    scope: TurnScope,
) -> TriggeredRunDeliveryRequest {
    TriggeredRunDeliveryRequest {
        run_id,
        scope,
        creator_user_id: fire.creator_user_id.clone(),
        project_scoped: fire.project_id.is_some(),
        prompt: fire.prompt.clone(),
        delivery_target: None,
        trigger_context: ironclaw_outbound::TriggerCommunicationContext {
            trigger_origin_ref: ironclaw_outbound::TriggerOriginRef::new(
                fire.identity.trigger_id().to_string(),
            )
            .expect("trigger origin ref"), // safety: trigger ids are valid origin refs.
            trigger_source_kind: ironclaw_outbound::TriggerSourceKind::Schedule,
            fire_slot: ironclaw_outbound::TriggerFireSlot::new(
                fire.identity.fire_slot().to_rfc3339(),
            )
            .expect("fire slot"), // safety: RFC3339 timestamps are valid fire slots.
        },
    }
}

async fn wait_for_post_messages_matching(
    egress: &RecordingEgress,
    description: &str,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> Vec<serde_json::Value> {
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let matches: Vec<serde_json::Value> = egress
                .requests()
                .into_iter()
                .filter(|request| request.url.ends_with("/api/chat.postMessage"))
                .filter_map(|request| serde_json::from_slice(&request.body).ok())
                .filter(|payload: &serde_json::Value| predicate(payload))
                .collect();
            if !matches.is_empty() {
                return matches;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    outcome.unwrap_or_else(|_| panic!("driver posts {description} within 5 s")) // safety: test-only timeout; panic message is the failure diagnostic
}

/// Poll `egress`'s recorded requests until at least one Slack
/// `chat.postMessage` matching the approval-prompt shape (JSON `text` field
/// containing `"approve"` and `gate_ref`) has been captured, then return every
/// such match. See `wait_for_post_messages_matching` for the shared
/// retry/backoff/timeout shape and why filtering by shape (not raw count) is
/// deliberate.
async fn wait_for_approval_prompt_messages(
    egress: &RecordingEgress,
    gate_ref: &str,
) -> Vec<serde_json::Value> {
    wait_for_post_messages_matching(
        egress,
        &format!("the approval-prompt chat.postMessage naming gate {gate_ref}"),
        |payload| {
            payload["text"]
                .as_str()
                .is_some_and(|text| text.contains("approve") && text.contains(gate_ref))
        },
    )
    .await
}

/// Full trigger→gate→approve twin of the live canary, at the crate tier.
///
/// A triggered run (personal, foreign thread scope) blocks on approval. The
/// `TriggeredRunDeliveryDriver` — the production triggered-delivery hook — posts
/// the approval prompt to the creator's Slack DM through a fake protocol egress
/// and auto-records a delivered gate route into the store the inbound workflow
/// reads. When the human replies with bare `approve` in that DM, the events
/// route resolves the gate on the run's foreign scope via the DRIVER-recorded
/// route (not a hand-seeded one). This welds two production assemblies that are
/// otherwise pinned only in isolation: the triggered-delivery route recording
/// (`slack_delivery` cfg(test)) and the inbound delivered-route resolution
/// (`bare_approve_in_dm_resolves_gate_recorded_by_observer`).
///
/// Doubles substitute only at seams the production triggered factory
/// (`build_triggered_run_delivery_hook_from_parts`) fills: `egress` real, and a
/// no-op `binding_service`. The final-reply tail after `approve` is pinned
/// separately by `slack_approval_reply_resumes_and_delivers_final_reply`.
#[tokio::test]
async fn triggered_approval_prompt_route_resolves_dm_approve_on_foreign_scope() {
    // Non-shared harness: the inbound observer's own route writes go to a separate
    // store, so `harness.route_store` (the store the workflow reads) is written
    // ONLY by the triggered driver under test — mirroring the manual-seed variant
    // `bare_approve_in_dm_resolves_gate_on_foreign_scope_via_delivered_route`, but
    // with the route produced by a real `TriggeredRunDeliveryDriver`.
    let (harness, inner_approvals) = build_harness_for_delivered_route_tests().await;

    // Establish the DM conversation binding and a blocked run whose id the driver
    // will route. (In production the triggered run id comes from the trigger
    // submit; here we reuse the harness's blocked run so the inbound approve has a
    // concrete run to target.)
    let block_response = harness.post_event(DM_BLOCK).await;
    assert_eq!(block_response.status(), StatusCode::OK);
    harness.drain().await;
    let blocked_run_id = harness
        .coordinator
        .blocked_run_id()
        .expect("run must be blocked after DM_BLOCK"); // safety: E2E test assertion.

    let tenant = TenantId::new(TENANT).expect("tenant"); // safety: static test tenant id is valid.
    let user = UserId::new(USER).expect("user"); // safety: static test user id is valid.
    let foreign_scope = foreign_run_scope();

    // Seed the creator's personal DM preference so the triggered approval prompt
    // resolves to team T-A / channel D123 — the same DM the inbound approve uses.
    let outbound = Arc::new(in_memory_backed_outbound_state_store());
    let dm_target = dm_reply_target_binding_ref();
    outbound
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(tenant.clone(), user.clone()),
                final_reply_target: Some(dm_target.clone()),
                progress_target: None,
                approval_prompt_target: Some(dm_target.clone()),
                auth_prompt_target: None,
                default_modality: None,
                updated_at: chrono::Utc::now(),
                updated_by: user.clone(),
            },
            expected_version: None,
        })
        .await
        .expect("seed personal preference"); // safety: in-memory store should not fail.

    let threads = InMemorySessionThreadService::default();

    let template = turn_state(
        foreign_scope.clone(),
        TurnActor::new(user.clone()),
        blocked_run_id,
        TurnStatePhase {
            status: TurnStatus::BlockedApproval,
            origin: TurnOriginKind::ScheduledTrigger,
            gate_ref: Some(GateRef::new(GATE).expect("gate ref")), // safety: static test gate ref is valid.
        },
        dm_target,
        AcceptedMessageRef::new("slack:triggered-approval").expect("accepted ref"), // safety: static test accepted ref is valid.
    );
    let coordinator: Arc<dyn TurnCoordinator> = Arc::new(ScriptedTriggerCoordinator::new(template));

    let outbound_store: Arc<dyn OutboundStateStorePort> = outbound.clone();
    let preferences: Arc<dyn CommunicationPreferenceRepository> = outbound;
    let fixture = triggered_delivery_fixture(Arc::clone(&outbound_store)).await;
    let driver_egress = fixture.driver_egress.clone();
    let services = RunDeliveryServices {
        binding_service: Arc::new(NoopTriggeredBindingService),
        thread_service: Arc::new(threads),
        turn_coordinator: coordinator,
        outbound_store,
        // Shared with the workflow's delivered-route index so the driver-recorded
        // route is what the inbound approve resolves against.
        route_store: harness.route_store.clone(),
        communication_preferences: preferences,
        coordinator: Arc::clone(&fixture.delivery_coordinator),
        extension_id: "slack".to_string(),
        fallback_notice_scope: test_fallback_notice_scope(),
        approval_context: None,
        blocked_auth_prompts: None,
        auth_flow_cancel: None,
    };
    let driver = TriggeredRunDeliveryDriver::with_event_router(
        services,
        Arc::new(in_memory_backed_outbound_state_store()),
        Arc::new(SlackTriggeredTargetResolver),
        AgentId::new(AGENT).expect("agent"), // safety: static test agent id is valid.
        Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test()),
    );

    // Fire the trigger. creator == USER so the recorded route keys to the same
    // user the inbound DM resolves to; project None => personal (not denied).
    let fire = TriggerFire {
        identity: TriggerFireIdentity::new(tenant.clone(), TriggerId::new(), chrono::Utc::now()),
        creator_user_id: user.clone(),
        agent_id: None,
        project_id: None,
        prompt: "triggered approval prompt".to_string(),
        delivery_target: None,
    };
    driver
        .on_trigger_submitted(triggered_request_from_fire(
            &fire,
            blocked_run_id,
            foreign_scope,
        ))
        .await;

    // The driver recorded a delivered gate route into the shared store, keyed by
    // the creator, on the triggered run's foreign scope, and carrying the DM
    // conversation fingerprint the inbound approve keys on.
    let route = wait_for_gate_route_matching(
        harness.route_store.as_ref(),
        &tenant,
        &user,
        GATE,
        |record| record.scope.thread_id == foreign_run_scope().thread_id,
    )
    .await;
    assert_eq!(route.run_id, blocked_run_id);
    assert_eq!(
        route.scope.thread_id,
        foreign_run_scope().thread_id,
        "route carries the triggered run's foreign thread scope"
    );
    assert!(
        route
            .delivered_conversation_fingerprints
            .contains(&dm_conversation_fingerprint()),
        "driver route must carry the DM conversation fingerprint the inbound approve keys on; got {:?}",
        route.delivered_conversation_fingerprints
    );

    // The committed blocked event posts one approval prompt naming the gate to
    // the Slack DM.
    let approval_prompts = wait_for_approval_prompt_messages(&driver_egress, GATE).await;
    assert_eq!(
        approval_prompts.len(),
        1,
        "expected exactly one approval-prompt chat.postMessage; got {approval_prompts:?}"
    );
    let prompt_payload = &approval_prompts[0];
    assert_eq!(prompt_payload["channel"], CHANNEL);
    let prompt_text = prompt_payload["text"]
        .as_str()
        .expect("approval prompt body carries a text field");
    assert!(
        prompt_text.contains("approve") && prompt_text.contains(GATE),
        "approval prompt body must name the gate: {prompt_text}"
    );

    // Inbound bare `approve` in the DM resolves the gate on the run's FOREIGN
    // scope via the DRIVER-recorded route: list_pending on the DM returns []
    // (ForeignScopeApprovalService), the workflow falls back to the conversation
    // fingerprint index and finds the driver route.
    harness.ensure_scope_thread(&foreign_run_scope()).await;
    let approve_response = harness.post_event(DM_APPROVE).await;
    let approve_status = approve_response.status();
    let approve_body = approve_response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(
        approve_status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&approve_body)
    );
    harness.drain().await;

    let requests = inner_approvals.requests();
    assert_eq!(requests.len(), 1, "exactly one approval resolve request");
    assert_eq!(
        requests[0].scope.thread_id,
        foreign_run_scope().thread_id,
        "scope rewritten to the triggered run's foreign thread via the driver route"
    );
    assert_eq!(
        requests[0].run_id_hint,
        Some(blocked_run_id),
        "run_id_hint carries the driver-recorded route's run_id"
    );
    assert_eq!(
        requests[0].decision,
        ApprovalInteractionDecision::ApproveOnce
    );
}

/// Build a Slack shared-channel reply-target binding ref for team `T-A` /
/// channel `C123` — i.e. NOT a personal DM. `slack_reply_target_is_personal_dm`
/// requires the conversation id to start with `D`; `C123` fails that check by
/// construction. Used to drive `TriggeredRunDeliveryDriver` through its
/// send-time OAuth-DM backstop (`TriggeredNotificationFailure::OAuthTargetNotDm`):
/// an OAuth-carrying auth prompt whose resolved `auth_prompt_target` is not a
/// personal DM must never post the setup link.
fn non_dm_channel_reply_target_binding_ref() -> ReplyTargetBindingRef {
    fn seg(name: &str, value: &str) -> String {
        format!("{}:{}:{};", name, value.len(), value)
    }
    const NON_DM_CHANNEL: &str = "C123";
    let raw = format!(
        "{}{}{}{}{}{}{}{}{}",
        seg("adapter", SLACK_V2_ADAPTER_ID),
        seg("installation", INSTALLATION),
        seg("agent", AGENT),
        seg("project", ""),
        seg("space", TEAM),
        seg("conversation", NON_DM_CHANNEL),
        seg("topic", ""),
        seg("actor_kind", SLACK_USER_ACTOR_KIND),
        seg("actor", SLACK_USER),
    );
    ironclaw_slack_extension::slack_reply_target_binding_ref_from_raw(raw)
        .expect("channel reply target binding ref") // safety: static test binding ref is valid.
}

/// Poll `egress`'s recorded requests until at least one Slack `chat.postMessage`
/// matching the auth-prompt shape (JSON `text` field containing "Authentication
/// required" — the literal body `triggered_notification_for_state` sets for the
/// `BlockedAuth` arm) has been captured, then return every such match. See
/// `wait_for_post_messages_matching` for the shared bounded asynchronous wait.
async fn wait_for_auth_prompt_messages(egress: &RecordingEgress) -> Vec<serde_json::Value> {
    wait_for_post_messages_matching(
        egress,
        "the auth-prompt chat.postMessage (\"Authentication required\")",
        |payload| {
            payload["text"]
                .as_str()
                .is_some_and(|text| text.contains("Authentication required"))
        },
    )
    .await
}

/// Auth-gate twin of `triggered_approval_prompt_route_resolves_dm_approve_on_foreign_scope`:
/// a triggered run (personal, foreign thread scope) blocks on auth instead of
/// approval. `TriggeredRunDeliveryDriver` resolves the creator's `auth_prompt_target`
/// preference to their Slack DM and posts the OAuth setup link there — mirroring
/// the inbound DM auth-prompt assertion shape in
/// `slack_dm_delivers_auth_prompt_with_setup_link_after_immediate_ack`, but driven
/// through the triggered delivery path (a real `TriggeredRunDeliveryDriver`, no
/// inbound HTTP event) instead of an inbound message.
///
/// `TriggeredRunDeliveryDriver` only ever resolves to the creator's *personal*
/// target (never a channel — see its struct doc comment: "delivers the result to
/// the creator's personal Slack DM"), so there is no "channel" arm to mirror
/// `slack_channel_auth_prompt_omits_setup_link_after_immediate_ack` with here.
/// The discriminating negative arm instead exercises the driver's own DM-only
/// backstop, in
/// `triggered_auth_prompt_oauth_target_not_dm_suppresses_setup_link_and_cancels_run`
/// below: when the resolved auth-prompt target is not a personal DM, the setup
/// link must never be posted and the run must be cancelled instead.
#[tokio::test]
async fn triggered_auth_prompt_route_delivers_dm_setup_link_on_foreign_scope() {
    let tenant = TenantId::new(TENANT).expect("tenant"); // safety: static test tenant id is valid.
    let user = UserId::new(USER).expect("user"); // safety: static test user id is valid.
    let foreign_scope = foreign_run_scope();
    let run_id = TurnRunId::new();

    // Seed the creator's personal auth-prompt preference so the triggered auth
    // prompt resolves to team T-A / channel D123 — a personal DM.
    let outbound = Arc::new(in_memory_backed_outbound_state_store());
    let dm_target = dm_reply_target_binding_ref();
    outbound
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(tenant.clone(), user.clone()),
                final_reply_target: Some(dm_target.clone()),
                progress_target: None,
                approval_prompt_target: None,
                auth_prompt_target: Some(dm_target.clone()),
                default_modality: None,
                updated_at: chrono::Utc::now(),
                updated_by: user.clone(),
            },
            expected_version: None,
        })
        .await
        .expect("seed personal preference"); // safety: in-memory store should not fail.

    let threads = InMemorySessionThreadService::default();

    let template = turn_state(
        foreign_scope.clone(),
        TurnActor::new(user.clone()),
        run_id,
        TurnStatePhase {
            status: TurnStatus::BlockedAuth,
            origin: TurnOriginKind::ScheduledTrigger,
            gate_ref: Some(GateRef::new(AUTH_GATE).expect("auth gate ref")), // safety: static test gate ref is valid.
        },
        dm_target,
        AcceptedMessageRef::new("slack:triggered-auth").expect("accepted ref"), // safety: static test accepted ref is valid.
    );
    let coordinator: Arc<dyn TurnCoordinator> = Arc::new(ScriptedTriggerCoordinator::new(template));

    let auth_provider = Arc::new(FakeAuthChallengeProvider::default());
    let auth_challenges: Arc<dyn AuthChallengeProvider> = auth_provider.clone();

    let outbound_store: Arc<dyn OutboundStateStorePort> = outbound.clone();
    let preferences: Arc<dyn CommunicationPreferenceRepository> = outbound;
    let route_store: Arc<dyn DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let fixture = triggered_delivery_fixture(Arc::clone(&outbound_store)).await;
    let driver_egress = fixture.driver_egress.clone();
    let services = RunDeliveryServices {
        binding_service: Arc::new(NoopTriggeredBindingService),
        thread_service: Arc::new(threads),
        turn_coordinator: coordinator,
        outbound_store,
        route_store: route_store.clone(),
        communication_preferences: preferences,
        coordinator: Arc::clone(&fixture.delivery_coordinator),
        extension_id: "slack".to_string(),
        fallback_notice_scope: test_fallback_notice_scope(),
        approval_context: None,
        blocked_auth_prompts: Some(Arc::new(ProductAuthBlockedAuthPromptSource::new(Some(
            auth_challenges,
        ))) as Arc<dyn BlockedAuthPromptSource>),
        auth_flow_cancel: None,
    };
    let driver = TriggeredRunDeliveryDriver::with_event_router(
        services,
        Arc::new(in_memory_backed_outbound_state_store()),
        Arc::new(SlackTriggeredTargetResolver),
        AgentId::new(AGENT).expect("agent"), // safety: static test agent id is valid.
        Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test()),
    );

    let fire = TriggerFire {
        identity: TriggerFireIdentity::new(tenant.clone(), TriggerId::new(), chrono::Utc::now()),
        creator_user_id: user.clone(),
        agent_id: None,
        project_id: None,
        prompt: "triggered auth prompt".to_string(),
        delivery_target: None,
    };
    driver
        .on_trigger_submitted(triggered_request_from_fire(&fire, run_id, foreign_scope))
        .await;

    // The committed blocked event posts the auth prompt and OAuth setup link
    // to the creator's Slack DM.
    let auth_prompts = wait_for_auth_prompt_messages(&driver_egress).await;
    assert_eq!(
        auth_prompts.len(),
        1,
        "expected exactly one auth-prompt chat.postMessage; got {auth_prompts:?}"
    );
    let prompt_payload = &auth_prompts[0];
    assert_eq!(prompt_payload["channel"], CHANNEL);
    let prompt_text = prompt_payload["text"]
        .as_str()
        .expect("auth prompt body carries a text field");
    assert!(
        prompt_text.contains("Authentication required"),
        "auth prompt body must name the auth requirement: {prompt_text}"
    );
    assert!(
        prompt_text.contains("Setup link: https://provider.example/oauth"),
        "auth prompt body must carry the OAuth setup link when resolved to the \
         creator's personal DM: {prompt_text}"
    );
    auth_provider.assert_single_call();
}

/// Discriminating negative arm for
/// `triggered_auth_prompt_route_delivers_dm_setup_link_on_foreign_scope` (see its
/// doc comment for why a "channel" arm does not apply to
/// `TriggeredRunDeliveryDriver`). When the creator's `auth_prompt_target`
/// preference resolves to a non-DM target, the send-time OAuth-DM backstop
/// (`require_direct_message_target` in `deliver_triggered_notification`) must
/// reject the OAuth-carrying prompt before it is ever posted — the setup link is
/// never leaked to a shared channel. `deliver_triggered_run` handles the
/// resulting `OAuthTargetNotDm` failure by cancelling the blocked run and posting
/// the plain-text auth-unavailable notice (`SLACK_AUTH_UNAVAILABLE_MESSAGE`)
/// instead, using `final_reply_target` (still the DM here) so the notice itself
/// is still observable.
#[tokio::test]
async fn triggered_auth_prompt_oauth_target_not_dm_suppresses_setup_link_and_cancels_run() {
    let tenant = TenantId::new(TENANT).expect("tenant"); // safety: static test tenant id is valid.
    let user = UserId::new(USER).expect("user"); // safety: static test user id is valid.
    let foreign_scope = foreign_run_scope();
    let run_id = TurnRunId::new();

    // auth_prompt_target resolves to a shared channel (not a DM); final_reply_target
    // stays the DM so the follow-up deny notice can still be delivered and inspected.
    let outbound = Arc::new(in_memory_backed_outbound_state_store());
    let dm_target = dm_reply_target_binding_ref();
    let channel_target = non_dm_channel_reply_target_binding_ref();
    outbound
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(tenant.clone(), user.clone()),
                final_reply_target: Some(dm_target.clone()),
                progress_target: None,
                approval_prompt_target: None,
                auth_prompt_target: Some(channel_target),
                default_modality: None,
                updated_at: chrono::Utc::now(),
                updated_by: user.clone(),
            },
            expected_version: None,
        })
        .await
        .expect("seed personal preference"); // safety: in-memory store should not fail.

    let threads = InMemorySessionThreadService::default();

    let template = turn_state(
        foreign_scope.clone(),
        TurnActor::new(user.clone()),
        run_id,
        TurnStatePhase {
            status: TurnStatus::BlockedAuth,
            origin: TurnOriginKind::ScheduledTrigger,
            gate_ref: Some(GateRef::new(AUTH_GATE).expect("auth gate ref")), // safety: static test gate ref is valid.
        },
        dm_target,
        AcceptedMessageRef::new("slack:triggered-auth-not-dm").expect("accepted ref"), // safety: static test accepted ref is valid.
    );
    let coordinator = Arc::new(ScriptedTriggerCoordinator::new(template));

    let auth_provider = Arc::new(FakeAuthChallengeProvider::default());
    let auth_challenges: Arc<dyn AuthChallengeProvider> = auth_provider.clone();

    let outbound_store: Arc<dyn OutboundStateStorePort> = outbound.clone();
    let preferences: Arc<dyn CommunicationPreferenceRepository> = outbound;
    let route_store: Arc<dyn DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let fixture = triggered_delivery_fixture(Arc::clone(&outbound_store)).await;
    let driver_egress = fixture.driver_egress.clone();
    let services = RunDeliveryServices {
        binding_service: Arc::new(NoopTriggeredBindingService),
        thread_service: Arc::new(threads),
        turn_coordinator: Arc::clone(&coordinator) as Arc<dyn TurnCoordinator>,
        outbound_store,
        route_store: route_store.clone(),
        communication_preferences: preferences,
        coordinator: Arc::clone(&fixture.delivery_coordinator),
        extension_id: "slack".to_string(),
        fallback_notice_scope: test_fallback_notice_scope(),
        approval_context: None,
        blocked_auth_prompts: Some(Arc::new(ProductAuthBlockedAuthPromptSource::new(Some(
            auth_challenges,
        ))) as Arc<dyn BlockedAuthPromptSource>),
        auth_flow_cancel: None,
    };
    let driver = TriggeredRunDeliveryDriver::with_event_router(
        services,
        Arc::new(in_memory_backed_outbound_state_store()),
        Arc::new(SlackTriggeredTargetResolver),
        AgentId::new(AGENT).expect("agent"), // safety: static test agent id is valid.
        Arc::new(RunDeliveryEventRouter::new_ephemeral_for_test()),
    );

    let fire = TriggerFire {
        identity: TriggerFireIdentity::new(tenant.clone(), TriggerId::new(), chrono::Utc::now()),
        creator_user_id: user.clone(),
        agent_id: None,
        project_id: None,
        prompt: "triggered auth prompt not dm".to_string(),
        delivery_target: None,
    };
    driver
        .on_trigger_submitted(triggered_request_from_fire(&fire, run_id, foreign_scope))
        .await;

    // The OAuth target is not a DM, so the handler cancels the run and emits
    // only the fixed auth-unavailable notice.
    let messages =
        wait_for_post_messages_matching(&driver_egress, "at least one chat.postMessage", |_| true)
            .await;
    assert_eq!(
        messages.len(),
        1,
        "expected exactly one chat.postMessage — the auth-unavailable deny notice; \
         the OAuth-carrying prompt must never be posted to a non-DM target; got {messages:?}"
    );
    let text = messages[0]["text"]
        .as_str()
        .expect("deny notice carries a text field");
    assert!(
        !text.contains("Setup link:") && !text.contains("https://provider.example/oauth"),
        "OAuth setup link must never be posted to a non-DM target: {text}"
    );
    assert!(
        text.contains("Ironclaw web app"),
        "expected the auth-unavailable deny notice, got: {text}"
    );
    assert_eq!(
        coordinator.cancel_call_count(),
        1,
        "the blocked run must be cancelled exactly once when the OAuth target is not a DM"
    );
    auth_provider.assert_single_call();
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
/// resolves the most-recently-delivered gate (recency tiebreak) rather than
/// failing closed. Exactly one resolve is forwarded — for the newest route —
/// and `approve gate:<ref>` remains available to target a specific gate.
#[tokio::test]
async fn bare_approve_with_two_live_routes_resolves_most_recent() {
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
            // Older delivery — recency must prefer GATE_B below.
            recorded_at: chrono::Utc::now() - chrono::Duration::hours(1),
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
    harness.ensure_scope_thread(&foreign_run_scope()).await;
    let approve_response = harness.post_event(DM_APPROVE).await;
    assert_eq!(approve_response.status(), StatusCode::OK);
    harness.drain().await;

    // Exactly one resolve is forwarded — for the most-recently-delivered route
    // (GATE_B) — rather than fanning out or failing closed without consulting the
    // service.
    let requests = inner_approvals.requests();
    assert_eq!(
        requests.len(),
        1,
        "recency must forward exactly one resolve, got {}",
        requests.len()
    );
    assert_eq!(
        requests[0].gate_ref.as_str(),
        GATE_B,
        "recency must resolve the most-recently-delivered gate"
    );

    // No ambiguous hint: the only message is the approval prompt posted by the
    // DM_BLOCK drain. The bare approve resolved cleanly, so nothing else is posted.
    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        1,
        "expected only the DM_BLOCK approval prompt, got {} message(s)",
        messages.len()
    );
}

/// Bare `approve` in the DM with ONE approval gate AND one stale/uncompleted
/// auth gate both delivered to the same DM resolves the approval gate —
/// NOT AmbiguousGate.
///
/// Scenario: a run first triggered an auth gate (e.g. OAuth not yet completed,
/// still live in the store) and later a second run triggered an approval gate,
/// both delivered to the same DM.  The user sends a bare "approve".
/// `list_pending` returns [] (ForeignScopeApprovalService).  The workflow falls
/// back to the conversation-fingerprint index and finds TWO records.  Before
/// this fix, both records counted toward `live.len()` → `Ambiguous` → error.
/// After this fix, the approval-path gate-kind filter drops the auth record,
/// leaving exactly one approval record → `Single` → resolved successfully.
///
/// This test would fail on the pre-fix code path: the auth-gate record would
/// inflate `live.len()` to 2 and trigger `AmbiguousGate`.
#[tokio::test]
async fn bare_approve_with_one_approval_and_one_stale_auth_gate_resolves_approval() {
    let (harness, inner_approvals) = build_harness_for_delivered_route_tests().await;

    // Submit a turn so the DM conversation binding is created.
    let block_response = harness.post_event(DM_BLOCK).await;
    assert_eq!(block_response.status(), StatusCode::OK);
    harness.drain().await;
    let blocked_run_id = harness
        .coordinator
        .blocked_run_id()
        .expect("run must be blocked after DM_BLOCK"); // safety: E2E test assertion.

    let fingerprint = dm_conversation_fingerprint();

    // Seed the approval-gate route record (the "real" pending gate the user
    // wants to resolve).
    harness
        .route_store
        .record_delivered_gate_route(ironclaw_outbound::DeliveredGateRouteRecord {
            tenant_id: TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            user_id: UserId::new(USER).expect("user"), // safety: static test user id is valid.
            gate_ref: GATE.to_string(), // gate:approval-... prefix — is_approval_gate_ref → true
            run_id: blocked_run_id,
            scope: foreign_run_scope(),
            recorded_at: chrono::Utc::now(),
            delivered_conversation_fingerprints: vec![fingerprint.clone()],
        })
        .await
        .expect("approval route record write"); // safety: in-memory store should not fail.

    // Seed a stale/uncompleted auth-gate route record in the SAME conversation.
    // This simulates a lingering `gate:auth-*` record that was never completed
    // (e.g. the user dismissed the OAuth flow without finishing it).  Because
    // the 48h TTL has not elapsed it is still "live" and would previously
    // contaminate the approval bare-resolve lookup.
    harness
        .route_store
        .record_delivered_gate_route(ironclaw_outbound::DeliveredGateRouteRecord {
            tenant_id: TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            user_id: UserId::new(USER).expect("user"), // safety: static test user id is valid.
            gate_ref: AUTH_GATE.to_string(), // gate:auth-... prefix — is_auth_gate_ref → true
            run_id: ironclaw_turns::TurnRunId::new(),
            scope: foreign_run_scope(),
            recorded_at: chrono::Utc::now(),
            delivered_conversation_fingerprints: vec![fingerprint],
        })
        .await
        .expect("auth route record write"); // safety: in-memory store should not fail.

    // Post a bare "approve".  Two records exist in the conversation bucket but
    // only the approval-gate record passes the gate-kind filter, so the workflow
    // should resolve Single → forward exactly one approval resolve request.
    harness.ensure_scope_thread(&foreign_run_scope()).await;
    let approve_response = harness.post_event(DM_APPROVE).await;
    assert_eq!(approve_response.status(), StatusCode::OK);
    harness.drain().await;

    let requests = inner_approvals.requests();
    assert_eq!(
        requests.len(),
        1,
        "exactly one approval resolve must be forwarded — auth gate must be filtered out; got {} request(s)",
        requests.len()
    );
    assert_eq!(
        requests[0].run_id_hint,
        Some(blocked_run_id),
        "run_id_hint must come from the approval route record"
    );
    assert_eq!(
        requests[0].gate_ref.as_str(),
        GATE,
        "resolved gate_ref must be the approval gate"
    );
    assert_eq!(
        requests[0].decision,
        ApprovalInteractionDecision::ApproveOnce
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
    let harness = build_harness(TurnMode::Complete {
        assistant_text: "hello personal Slack binding".into(),
    })
    .await;

    let response = harness.post_event(dm_message("Ev-identity", "hello")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_body(response, "ok").await;
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], CHANNEL);
    assert_eq!(messages[0]["text"], "hello personal Slack binding");
    // The generic assembly resolves the verified actor through the
    // channel-identity binding store with the installation-scoped key, then
    // performs an uncached freshness read before submitting the turn. The
    // second read keeps a revoked positive cache entry from authorizing one
    // more inbound message.
    let expected_lookup = ("slack".to_string(), format!("{INSTALLATION}:{SLACK_USER}"));
    assert_eq!(
        harness.identity_lookup.calls(),
        vec![expected_lookup.clone(), expected_lookup],
        "inbound actor resolution and freshness validation must consult the identity lookup"
    );
}

/// Generic shared-channel admission (§5.3): an unconfigured shared channel
/// fails closed (no turn, no reply, vendor still gets its 2xx); saving the
/// channel into `slack_allowed_channels` admits the next event under the
/// managed derived subject (the retired lane's `user:slack-channel:{sha16}`
/// value shape); an explicit `slack_subject_routes` entry runs its channel
/// as the configured subject. Saves take effect per request — no rebuild.
#[tokio::test]
async fn shared_channel_admission_follows_saved_admin_configuration() {
    let harness = build_harness(TurnMode::Complete {
        assistant_text: "channel reply".into(),
    })
    .await;
    // C777 is not in the harness's saved allowed list: fail closed.
    let refused = harness.post_event(SHARED_CHANNEL_UNROUTED).await;
    assert_eq!(refused.status(), StatusCode::OK, "vendor keeps its 2xx");
    harness.drain().await;
    assert_eq!(
        harness.coordinator.submitted_turn_count(),
        0,
        "an unrouted shared channel must not reach the turn coordinator"
    );
    assert!(
        harness.slack_messages().is_empty(),
        "no reply may leak into an unadmitted shared channel"
    );

    // The operator admits C777 (fresh event id: the refused event settled
    // terminally in the durable idempotency ledger).
    harness
        .admin_configuration_resolver
        .configure_admin_group_for_test(
            "extension.slack",
            vec![(
                "slack_allowed_channels".to_string(),
                r#"["C123","C777"]"#.to_string(),
            )],
        )
        .await
        .expect("save allowed channels"); // safety: manifest declares the handle.
    let admitted = harness.post_event(SHARED_CHANNEL_ALLOWED).await;
    assert_eq!(admitted.status(), StatusCode::OK);
    harness.drain().await;
    let scopes = harness.coordinator.submitted_scopes();
    assert_eq!(scopes.len(), 1, "the admitted channel submits one turn");
    let expected_managed_subject =
        crate::extension_host::channel_subject_routes::managed_channel_subject_user_id(
            ADAPTER,
            &TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            &ironclaw_product::AdapterInstallationId::new(INSTALLATION).expect("installation"), // safety: static test installation id is valid.
            Some(TEAM),
            "C777",
        )
        .expect("managed subject derivation");
    assert_eq!(
        scopes[0].thread_owner.explicit_owner_user_id(),
        Some(&expected_managed_subject),
        "an allowed channel runs under the managed derived subject"
    );
    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["channel"], "C777");
    assert_eq!(messages[0]["text"], "channel reply");

    // An explicit subject route wins for its channel.
    harness
        .admin_configuration_resolver
        .configure_admin_group_for_test(
            "extension.slack",
            vec![(
                "slack_subject_routes".to_string(),
                r#"{"C888":"user:ops-agent"}"#.to_string(),
            )],
        )
        .await
        .expect("save subject routes"); // safety: manifest declares the handle.
    let routed = harness.post_event(SHARED_CHANNEL_ROUTED).await;
    assert_eq!(routed.status(), StatusCode::OK);
    harness.drain().await;
    let scopes = harness.coordinator.submitted_scopes();
    assert_eq!(scopes.len(), 2);
    assert_eq!(
        scopes[1]
            .thread_owner
            .explicit_owner_user_id()
            .map(|user| user.as_str()),
        Some("user:ops-agent"),
        "an explicit subject route runs its channel as the configured subject"
    );
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

/// The approval transition and the resolution acknowledgement can both
/// reference the same run, but committed lifecycle stages still emit one gate
/// prompt and one final reply. Acknowledgements do not create a second
/// delivery owner; run-stage and durable outbound claims make replay
/// idempotent.
#[tokio::test]
async fn approval_resolution_and_lifecycle_events_deliver_each_stage_once() {
    let harness = build_harness(TurnMode::BlockApproval).await;

    // The committed BlockedApproval state owns the prompt.
    let first = harness
        .post_event(dm_message("Ev-fanout-block", "needs approval fanout"))
        .await;
    assert_eq!(first.status(), StatusCode::OK);

    // Wait for asynchronous provider dispatch.
    for _ in 0..200 {
        if harness.slack_messages().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        1,
        "expected exactly one approval prompt before the approve event; got {}: {:?}",
        messages.len(),
        messages
    );
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Approval needed")),
        "first message must be the approval prompt; got {:?}",
        messages[0]["text"]
    );

    // Resolution commits Completed on that same run.
    let second = harness
        .post_event(dm_message("Ev-fanout-approve", "approve"))
        .await;
    assert_eq!(second.status(), StatusCode::OK);

    // The Completed stage owns exactly one final reply.
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        2,
        "expected exactly 2 messages: approval prompt + final reply, not {} (duplicate final reply was posted without the fix)",
        messages.len()
    );
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Approval needed")),
        "messages[0] must be the approval prompt"
    );
    assert_eq!(
        messages[1]["text"], "approved and finished",
        "messages[1] must be the final reply"
    );
}

#[tokio::test]
async fn slack_dm_delivers_auth_prompt_with_setup_link_after_immediate_ack() {
    let auth_provider = Arc::new(FakeAuthChallengeProvider::default());
    let auth_challenges: Arc<dyn AuthChallengeProvider> = auth_provider.clone();
    let harness =
        build_harness_with_auth_challenges(TurnMode::BlockAuth, Some(auth_challenges)).await;

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
    let harness =
        build_harness_with_auth_challenges(TurnMode::BlockAuth, Some(auth_challenges)).await;

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
    let harness =
        build_harness_with_auth_challenges(TurnMode::BlockAuth, Some(auth_challenges)).await;

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

/// A user can take longer than the retired delivery poll window to complete
/// OAuth in the provider browser. The durable run still resumes and completes,
/// so the originating external conversation must receive that final answer
/// from the later lifecycle event without a task waiting in between.
///
/// This is intentionally provider-neutral: Slack is only the recording
/// channel adapter used by this generic channel-host harness. The same run
/// lifecycle is exercised when a Gmail/Notion/MCP credential gate originated
/// from Telegram or another external channel.
#[tokio::test]
async fn external_channel_delivers_final_after_oauth_outlives_delivery_poll_window() {
    let auth_provider = Arc::new(FakeAuthChallengeProvider::default());
    let auth_challenges: Arc<dyn AuthChallengeProvider> = auth_provider.clone();
    let harness =
        build_harness_with_auth_challenges(TurnMode::BlockAuth, Some(auth_challenges)).await;

    let response = harness
        .post_event(dm_message("Ev-auth", "needs auth"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    for _ in 0..80 {
        if harness.slack_messages().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        1,
        "auth prompt must reach the source channel"
    );
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("Authentication required"))
    );

    // Model a real browser OAuth flow: the callback arrives after the retired
    // focused timeout while the durable run remains auth-blocked. Draining
    // proves no observer task is waiting to make the assertion pass; the
    // later Resumed/Completed facts independently own their notifications.
    tokio::time::sleep(Duration::from_millis(125)).await;
    harness.drain().await;
    harness
        .coordinator
        .resume_blocked_run_to_running()
        .await
        .expect("OAuth callback resumes the durable auth-blocked run");
    harness
        .coordinator
        .complete_active_run("authenticated after browser callback")
        .await
        .expect("resumed run completes");

    for _ in 0..40 {
        if harness.slack_messages().len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        3,
        "the completed run must be delivered after a delayed OAuth callback; got {messages:?}"
    );
    assert_eq!(messages[1]["channel"], CHANNEL);
    assert_eq!(messages[1]["text"], "Ironclaw is thinking...");
    assert_eq!(messages[2]["channel"], CHANNEL);
    assert_eq!(messages[2]["text"], "authenticated after browser callback");
    auth_provider.assert_single_call();
}

#[derive(Debug, Clone)]
enum TurnMode {
    Complete {
        assistant_text: String,
    },
    Running,
    BlockApproval,
    /// Starts as BlockedApproval; the test manually transitions to BlockedAuth
    /// via `RecordingTurnCoordinator::transition_blocked_approval_to_blocked_auth`.
    BlockApprovalThenAuth,
    BlockAuth,
}

#[derive(Clone)]
struct RecordingTurnCoordinator {
    state: Arc<Mutex<RecordingTurnState>>,
    threads: InMemorySessionThreadService,
    mode: TurnMode,
    events: Arc<RunDeliveryEventRouter>,
}

struct RecordingTurnState {
    runs: std::collections::HashMap<TurnRunId, TurnRunState>,
    active_run_id: Option<TurnRunId>,
    blocked_run_id: Option<TurnRunId>,
    submitted_turn_count: usize,
    submitted_scopes: Vec<TurnScope>,
}

impl RecordingTurnCoordinator {
    fn new(
        threads: InMemorySessionThreadService,
        mode: TurnMode,
        events: Arc<RunDeliveryEventRouter>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(RecordingTurnState {
                runs: std::collections::HashMap::new(),
                active_run_id: None,
                blocked_run_id: None,
                submitted_turn_count: 0,
                submitted_scopes: Vec::new(),
            })),
            threads,
            mode,
            events,
        }
    }

    async fn publish_state(
        &self,
        state: TurnRunState,
        kind: TurnEventKind,
    ) -> Result<(), ProductWorkflowError> {
        let run_id = state.run_id;
        self.events
            .publish(TurnLifecycleEvent::from_run_state(&state, kind, None))
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: error.to_string(),
            })?;
        self.events.wait_until_run_idle(run_id).await;
        Ok(())
    }

    async fn publish_current_state(
        &self,
        run_id: TurnRunId,
        kind: TurnEventKind,
    ) -> Result<(), ProductWorkflowError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .runs
            .get(&run_id)
            .cloned()
            .ok_or_else(|| ProductWorkflowError::TurnResumeRejected {
                reason: "missing run state".into(),
            })?;
        self.publish_state(state, kind).await
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

    fn set_run_origin(
        &self,
        run_id: TurnRunId,
        origin: TurnOriginKind,
    ) -> Result<(), ProductWorkflowError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let run = state.runs.get_mut(&run_id).ok_or_else(|| {
            ProductWorkflowError::TurnResumeRejected {
                reason: "missing run state".into(),
            }
        })?;
        let context = run.product_context.as_mut().ok_or_else(|| {
            ProductWorkflowError::TurnResumeRejected {
                reason: "missing product context".into(),
            }
        })?;
        context.origin = origin;
        Ok(())
    }

    /// Scopes of submitted turns in submission order — shared-channel
    /// admission assertions read the resolved subject (thread owner) here.
    fn submitted_scopes(&self) -> Vec<TurnScope> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .submitted_scopes
            .clone()
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
        origin: TurnOriginKind,
    ) -> Result<(), ProductWorkflowError> {
        append_final_assistant_message(&self.threads, &scope, run_id, text).await?;
        let completed = {
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
                    TurnStatePhase {
                        status: TurnStatus::Completed,
                        origin,
                        gate_ref: None,
                    },
                    reply_target_binding_ref,
                    accepted_message_ref,
                ),
            );
            state.runs.get(&run_id).cloned().ok_or_else(|| {
                ProductWorkflowError::TurnResumeRejected {
                    reason: "missing completed run state".into(),
                }
            })?
        };
        self.publish_state(completed, TurnEventKind::Completed)
            .await
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
        self.complete_run(scope, actor, run_id, text, TurnOriginKind::Inbound)
            .await
    }

    async fn resume_blocked_run_to_running(&self) -> Result<(), ProductWorkflowError> {
        let resumed = {
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
            state.runs.get(&run_id).cloned().ok_or_else(|| {
                ProductWorkflowError::TurnResumeRejected {
                    reason: "missing resumed run state".into(),
                }
            })?
        };
        self.publish_state(resumed, TurnEventKind::Resumed).await
    }

    /// Complete the blocked run to `Completed` in a single locked mutation, skipping
    /// any observable `Running` state.
    ///
    /// This prevents the lifecycle router from observing the intermediate gap between
    /// `resume_blocked_run_to_running` and `complete_active_run`, observing
    /// `Running` with no blocked marker, and posting the "Ironclaw is thinking..."
    /// working indicator — which would produce a spurious 4th message and make the
    /// `messages.len() == 3` assertion flaky.
    async fn complete_blocked_run(&self, text: &str) -> Result<(), ProductWorkflowError> {
        // Append the final assistant message first (does not touch `state`).
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
                        reason: "missing run actor".into(),
                    })?;
            (run.scope.clone(), actor, run_id)
        };
        // Write the final assistant message before taking the lock that marks
        // the run Completed so the lifecycle consumer sees consistent terminal state.
        append_final_assistant_message(&self.threads, &scope, run_id, text).await?;
        // Now atomically transition: BlockedAuth → Completed, clear blocked_run_id.
        let completed = {
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
                    TurnStatePhase {
                        status: TurnStatus::Completed,
                        origin: TurnOriginKind::Inbound,
                        gate_ref: None,
                    },
                    reply_target_binding_ref,
                    accepted_message_ref,
                ),
            );
            // Clear blocked_run_id — the run is now terminal.
            state.blocked_run_id = None;
            state.runs.get(&run_id).cloned().ok_or_else(|| {
                ProductWorkflowError::TurnResumeRejected {
                    reason: "missing completed blocked run state".into(),
                }
            })?
        };
        self.publish_state(completed, TurnEventKind::Completed)
            .await
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
            TurnMode::BlockApproval | TurnMode::BlockApprovalThenAuth => {
                TurnStatus::BlockedApproval
            }
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
        let mut run_state = turn_state(
            request.scope,
            request.actor,
            run_id,
            TurnStatePhase {
                status,
                origin: TurnOriginKind::Inbound,
                gate_ref,
            },
            request.reply_target_binding_ref,
            request.accepted_message_ref,
        );
        // Preserve the sealed surface metadata produced by the real inbound
        // workflow. Hard-coding every fake run as `Direct` masks the shared-
        // channel auth-prompt policy this whole-path harness is meant to test.
        run_state.product_context = request.product_context;
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.submitted_turn_count += 1;
            state.submitted_scopes.push(run_state.scope.clone());
            state.active_run_id = Some(run_id);
            if matches!(
                status,
                TurnStatus::BlockedApproval | TurnStatus::BlockedAuth
            ) {
                state.blocked_run_id = Some(run_id);
            }
            state.runs.insert(run_id, run_state.clone());
        }
        let kind = match status {
            TurnStatus::BlockedApproval | TurnStatus::BlockedAuth => TurnEventKind::Blocked,
            TurnStatus::Completed => TurnEventKind::Completed,
            _ => TurnEventKind::Submitted,
        };
        self.publish_state(run_state, kind)
            .await
            .map_err(|error| TurnError::Unavailable {
                reason: error.to_string(),
            })?;
        Ok(response)
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("approval test uses fake ApprovalInteractionService")
    }

    async fn retry_turn(
        &self,
        _request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        panic!("retry_turn is not used")
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let run = state
            .runs
            .get_mut(&request.run_id)
            .ok_or_else(|| TurnError::Unavailable {
                reason: "missing run state for cancel_run".into(),
            })?;
        // Preserve idempotent-cancel contract shape: a second cancel of an
        // already-Cancelled run reports `already_terminal: true` rather than
        // first-cancel semantics, so the fake doesn't mask caller differences
        // on the retry path.
        let already_terminal = matches!(run.status, TurnStatus::Cancelled);
        if !already_terminal {
            run.status = TurnStatus::Cancelled;
            run.gate_ref = None;
        }
        // Intentionally do NOT clear `blocked_run_id` here.
        // The lifecycle handler uses `cancel_run` for idempotent teardown (e.g.
        // auth-unavailable auto-deny). The `blocked_run_id` pointer must remain
        // set so that a subsequent inbound "auth deny" text command can still
        // resolve through `RecordingAuthInteractionService::resolve` →
        // `cancel_blocked_run`, which then clears `blocked_run_id` and posts
        // the confirmation. The run-stage claim prevents the same auth-unavailable
        // fact from being processed twice.
        Ok(CancelRunResponse {
            run_id: request.run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor::default(),
            already_terminal,
            actor: None,
        })
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
        // The run's own thread owner: DM turns run as the bound user,
        // admitted shared channels as their configured/managed subject.
        owner_user_id: scope.thread_owner.explicit_owner_user_id().cloned(),
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

struct TurnStatePhase {
    status: TurnStatus,
    origin: TurnOriginKind,
    gate_ref: Option<GateRef>,
}

fn turn_state(
    scope: TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
    phase: TurnStatePhase,
    reply_target_binding_ref: ReplyTargetBindingRef,
    accepted_message_ref: AcceptedMessageRef,
) -> TurnRunState {
    TurnRunState {
        scope,
        actor: Some(actor.clone()),
        turn_id: TurnId::new(),
        run_id,
        status: phase.status,
        accepted_message_ref,
        source_binding_ref: ironclaw_turns::SourceBindingRef::new("slack:source")
            .expect("source binding"), // safety: static test source binding is valid.
        reply_target_binding_ref,
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        model_usage: None,
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref: phase.gate_ref,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor::default(),
        product_context: Some(ProductTurnContext::new(
            phase.origin,
            Some(TurnSurfaceType::Direct),
            // The generic ingress graph stamps the manifest extension id as
            // the product adapter id. Vendor codec ids such as `slack_v2`
            // remain inside the adapter and must not leak into run routing.
            Some(RunOriginAdapter::new(ADAPTER).expect("adapter")), // safety: static adapter id is valid.
            TurnOwner::Personal {
                user: actor.user_id.clone(),
            },
        )),
        resume_disposition: None,
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
        // Check the run's current status: only surface an approval gate when the run
        // is actually blocked on approval (not when it has already transitioned to
        // BlockedAuth after resolve() advanced the gate for BlockApprovalThenAuth).
        let is_blocked_approval = {
            let state = self
                .coordinator
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .runs
                .get(&run_id)
                .is_some_and(|run| run.status == TurnStatus::BlockedApproval)
        };
        if !is_blocked_approval {
            return Ok(ListPendingApprovalsResponse {
                approvals: Vec::new(),
            });
        }
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
        // For BlockApprovalThenAuth mode: approval resolves by advancing the run to
        // BlockedAuth (not completing it). This exercises the real "approval→auth
        // hop" path the production lifecycle consumer must handle — the run is still
        // blocked, now on an auth gate instead of an approval gate.
        if matches!(self.coordinator.mode, TurnMode::BlockApprovalThenAuth) {
            {
                let mut state = self
                    .coordinator
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let run = state.runs.get_mut(&run_id).ok_or_else(|| {
                    ProductWorkflowError::TurnResumeRejected {
                        reason: "missing blocked run state".into(),
                    }
                })?;
                run.status = TurnStatus::BlockedAuth;
                run.gate_ref = Some(GateRef::new(AUTH_GATE).expect("auth gate ref")); // safety: static test gate ref is valid.
                // blocked_run_id stays set — the run is still blocked, now on auth.
            }
            self.coordinator
                .publish_current_state(run_id, TurnEventKind::Blocked)
                .await?;
            return Ok(ResolveApprovalInteractionResponse::Approved(
                ResumeTurnResponse {
                    run_id,
                    status: TurnStatus::BlockedAuth,
                    event_cursor: EventCursor::default(),
                },
            ));
        }
        // Default mode: approval resolves by completing the run.
        self.coordinator
            .complete_run(
                request.scope.clone(),
                request.actor.clone(),
                run_id,
                "approved and finished",
                TurnOriginKind::Inbound,
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

/// Records every policy-approved channel egress call and synthesizes Slack
/// Web API responses — the transport-seam analog of the old protocol-egress
/// recorder.
#[derive(Clone, Default)]
struct RecordingEgress {
    requests: Arc<Mutex<Vec<ApprovedChannelEgress>>>,
}

impl RecordingEgress {
    fn requests(&self) -> Vec<ApprovedChannelEgress> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn bodies_for(&self, path: &str) -> Vec<serde_json::Value> {
        self.requests()
            .into_iter()
            .filter(|request| request.url.ends_with(path))
            .map(|request| {
                serde_json::from_slice(&request.body).expect("channel JSON body") // safety: channel adapters emit JSON request bodies in this test.
            })
            .collect()
    }
}

#[async_trait]
impl ChannelEgressTransport for RecordingEgress {
    async fn execute(
        &self,
        approved: ApprovedChannelEgress,
    ) -> Result<ironclaw_host_api::RestrictedEgressResponse, ironclaw_host_api::RestrictedEgressError>
    {
        let response = channel_response_for_approved(&approved);
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(approved);
        Ok(response)
    }
}

fn channel_response_for_approved(
    approved: &ApprovedChannelEgress,
) -> ironclaw_host_api::RestrictedEgressResponse {
    fn response(body: &[u8]) -> ironclaw_host_api::RestrictedEgressResponse {
        ironclaw_host_api::RestrictedEgressResponse {
            status: 200,
            body: body.to_vec(),
        }
    }
    let path = url::Url::parse(&approved.url)
        .map(|url| url.path().to_string())
        .unwrap_or_default();
    if path.starts_with("/api/chat.") {
        let has_json_content_type = approved.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-type") && value.starts_with("application/json")
        });
        if !has_json_content_type {
            return response(br#"{"ok":false,"error":"missing_post_type"}"#);
        }
    }
    if path == "/api/chat.postMessage" {
        let body: serde_json::Value = match serde_json::from_slice(&approved.body) {
            Ok(body) => body,
            Err(_) => {
                return response(br#"{"ok":false,"error":"invalid_json"}"#);
            }
        };
        let channel = body["channel"].as_str().unwrap_or("DTEST");
        let ts_seed = stable_slack_test_ts(&approved.body);
        return response(
            serde_json::json!({
                "ok": true,
                "channel": channel,
                "ts": ts_seed,
            })
            .to_string()
            .as_bytes(),
        );
    }
    if path.ends_with("/sendMessage") {
        return response(br#"{"ok":true,"result":{"message_id":4242}}"#);
    }
    response(br#"{"ok":true}"#)
}

fn stable_slack_test_ts(body: &[u8]) -> String {
    let mut hash = 0_u64;
    for byte in body {
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(*byte));
    }
    format!("1710000001.{:06}", hash % 1_000_000)
}

#[derive(Debug, Default)]
struct RecordingUserIdentityLookup {
    bindings: std::collections::HashMap<String, UserId>,
    calls: Mutex<Vec<(String, String)>>,
}

impl RecordingUserIdentityLookup {
    fn new(bindings: impl IntoIterator<Item = (String, UserId)>) -> Self {
        Self {
            bindings: bindings.into_iter().collect(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl RebornUserIdentityLookup for RecordingUserIdentityLookup {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((provider.to_string(), provider_user_id.to_string()));
        if provider != "slack" {
            return Ok(None);
        }
        Ok(self.bindings.get(provider_user_id).cloned())
    }

    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        if provider != "slack" {
            return Ok(false);
        }
        Ok(self.bindings.values().any(|bound| bound == user_id))
    }
}

fn dm_message(event_id: &'static str, text: &'static str) -> &'static str {
    match (event_id, text) {
        ("Ev-final", "hello") => DM_FINAL,
        ("Ev-approval", "needs approval") => DM_APPROVAL,
        ("Ev-block", "needs approval") => DM_BLOCK,
        ("Ev-approve", "approve") => DM_APPROVE,
        ("Ev-approve-explicit", "approve gate:approval-00000000-0000-0000-0000-000000000001") => {
            DM_APPROVE_EXPLICIT_GATE
        }
        ("Ev-forged", "hello") => DM_FORGED,
        ("Ev-identity", "hello") => DM_IDENTITY,
        ("Ev-working", "think") => DM_WORKING,
        ("Ev-auth", "needs auth") => DM_AUTH,
        // Gate-fanout regression fixtures
        ("Ev-fanout-block", "needs approval fanout") => DM_FANOUT_BLOCK,
        ("Ev-fanout-approve", "approve") => DM_FANOUT_APPROVE,
        // Approval→auth sequential gate fixture
        ("Ev-approval-then-auth-block", "needs approval then auth") => DM_APPROVAL_THEN_AUTH_BLOCK,
        ("Ev-approval-then-auth-approve", "approve") => DM_APPROVAL_THEN_AUTH_APPROVE,
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

const DM_WORKING: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-working",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"think","ts":"1710000000.000009"}
	}"#;

const DM_AUTH: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-auth",
	  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"needs auth","ts":"1710000000.000007"}
	}"#;

// ── Shared-channel admission fixtures ────────────────────────────────────────
// Used by `shared_channel_admission_follows_saved_admin_configuration`. C777/C888
// are outside the harness's default allowed list; distinct event ids keep
// the terminally-settled refusal out of the admitted replays.

const SHARED_CHANNEL_UNROUTED: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-shared-unrouted",
  "event":{"type":"app_mention","user":"U123","channel":"C777","text":"<@UBOT> hello","ts":"1710000003.000001"}
}"#;

const SHARED_CHANNEL_ALLOWED: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-shared-allowed",
  "event":{"type":"app_mention","user":"U123","channel":"C777","text":"<@UBOT> hello again","ts":"1710000003.000002"}
}"#;

const SHARED_CHANNEL_ROUTED: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-shared-routed",
  "event":{"type":"app_mention","user":"U123","channel":"C888","text":"<@UBOT> route me","ts":"1710000003.000003"}
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

/// Explicit gate-ref approve in the DM: `approve gate:approval-00000000-0000-0000-0000-000000000001`.
/// The gate ref token after "approve " is GATE (a valid `gate:approval-` prefixed ref).
/// Used by the delivered-gate-route test that verifies explicit gate ref resolves
/// directly (binding found → no cross-scope rewrite).
const DM_APPROVE_EXPLICIT_GATE: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-approve-explicit",
  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"approve gate:approval-00000000-0000-0000-0000-000000000001","ts":"1710000000.000005"}
}"#;

// ── Gate-fanout regression fixtures ──────────────────────────────────────────
// Used by `gate_prompt_is_posted_exactly_once_when_approval_ack_races_live_delivery_loop`.
// Distinct event_ids avoid idempotency-ledger collisions with all other fixtures.

/// User message that triggers a BlockApproval turn (gate-fanout regression).
const DM_FANOUT_BLOCK: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-fanout-block",
  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"needs approval fanout","ts":"1710000002.000001"}
}"#;

/// Approve event for the gate-fanout regression (resolves the BlockApproval gate).
const DM_FANOUT_APPROVE: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-fanout-approve",
  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"approve","ts":"1710000002.000002"}
}"#;

// ── Auth-resolution fanout regression fixtures ────────────────────────────────
// Used by `auth_prompt_is_posted_exactly_once_when_auth_resolution_ack_races_live_delivery_loop`.
// Distinct event_ids avoid idempotency-ledger collisions with all other fixtures.

/// User message that triggers a BlockAuth turn (auth-fanout regression).
const DM_AUTH_FANOUT_BLOCK: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-auth-fanout-block",
  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"needs auth fanout","ts":"1710000003.000001"}
}"#;

// ── Approval→Auth sequential gate fixture ────────────────────────────────────
// Used by `slack_approval_then_auth_resume_completes_without_second_approval`.
// Distinct event_id avoids idempotency-ledger collisions with all other fixtures.

/// User message that triggers a `BlockApprovalThenAuth` turn.
const DM_APPROVAL_THEN_AUTH_BLOCK: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-approval-then-auth-block",
  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"needs approval then auth","ts":"1710000004.000001"}
}"#;

/// Approve event for the approval→auth sequential gate regression.
/// Distinct event_id avoids idempotency-ledger collisions with DM_FANOUT_APPROVE.
const DM_APPROVAL_THEN_AUTH_APPROVE: &str = r#"{
  "type":"event_callback",
  "team_id":"T-A",
  "api_app_id":"A-slack",
  "event_id":"Ev-approval-then-auth-approve",
  "event":{"type":"message","channel_type":"im","user":"U123","channel":"D123","text":"approve","ts":"1710000004.000002"}
}"#;

/// Build a `ProductInboundEnvelope` carrying an `AuthResolution(CallbackCompleted)` payload.
///
/// Mirrors the shape that the WebUI gate-resolve endpoint would produce when an
/// OAuth callback completes and calls `observe_workflow_ack` directly (not via
/// any Slack text command — the Slack adapter has no "auth allow" syntax).
fn auth_resolution_allowed_envelope(callback_ref: &str) -> ProductInboundEnvelope {
    let adapter_id = ProductAdapterId::new(ADAPTER).expect("adapter id"); // safety: static test adapter id is valid.
    let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation id"); // safety: static test installation id is valid.
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: SLACK_SIGNATURE_HEADER.to_string(),
        },
        installation_id.as_str(),
    );
    let context = TrustedInboundContext::from_verified_evidence(
        adapter_id,
        installation_id,
        chrono::Utc::now(),
        &evidence,
    )
    .expect("trusted context"); // safety: static test context is valid.
    let payload = ProductInboundPayload::AuthResolution(
        AuthResolutionPayload::new(
            AUTH_GATE,
            AuthResolutionResult::CallbackCompleted {
                callback_ref: callback_ref.to_string(),
            },
        )
        .expect("auth resolution payload"), // safety: static test auth gate ref is valid.
    );
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new("evt:auth-fanout-resolve").expect("event id"), // safety: static test event id is valid.
        ExternalActorRef::new(SLACK_USER_ACTOR_KIND, SLACK_USER, None::<String>)
            .expect("actor ref"), // safety: static test actor ref is valid.
        ExternalConversationRef::new(Some(TEAM), CHANNEL, None, None).expect("conversation ref"), // safety: static test conversation ref is valid.
        payload,
    )
    .expect("parsed inbound"); // safety: static test inbound is valid.
    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope") // safety: static test envelope is valid.
}

/// Build a harness for auth-fanout tests and return the assembly-registered
/// post-admission observer alongside it.
///
/// The observer is needed because `AuthResolution(Allowed)` does not arrive
/// via a channel text command — it arrives from the WebUI gate-resolve path,
/// which drives the SAME observer instance the registered sink runs.
async fn build_harness_for_auth_fanout_test() -> (Harness, Arc<dyn PostAdmissionObserver>) {
    let auth_provider = Arc::new(FakeAuthChallengeProvider::default());
    let mut options = HarnessOptions::new(TurnMode::BlockAuth);
    options.auth_challenges = Some(auth_provider as Arc<dyn AuthChallengeProvider>);
    let harness = build_harness_with_options(options).await;
    let observer = harness
        .assembly
        .post_admission_observer_for_extension_for_test("slack")
        .expect("assembly registered the slack observer"); // safety: harness delivery deps are always present.
    (harness, observer)
}

#[tokio::test]
async fn auth_resolution_and_lifecycle_events_deliver_each_stage_once() {
    let (harness, observer) = build_harness_for_auth_fanout_test().await;

    // The committed BlockedAuth state owns the prompt.
    let first = harness.post_event(DM_AUTH_FANOUT_BLOCK).await;
    assert_eq!(first.status(), StatusCode::OK);

    // Wait for asynchronous provider dispatch.
    for _ in 0..200 {
        if harness.slack_messages().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        1,
        "expected exactly one auth prompt before the auth-resolution ack; got {}: {:?}",
        messages.len(),
        messages
    );
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Authentication required")),
        "first message must be the auth prompt; got {:?}",
        messages[0]["text"]
    );

    // Get the run_id of the blocked run so we can build a matching ack.
    let blocked_run_id = harness
        .coordinator
        .blocked_run_id()
        .expect("run must be blocked after auth-fanout message"); // safety: E2E test assertion.

    // Inject an `AuthResolution(Allowed)` ack directly — this simulates the
    // WebUI gate-resolve path (not a Slack text command). It references the
    // existing run but does not create another delivery owner.
    let auth_ack = ProductInboundAck::Accepted {
        accepted_message_ref: AcceptedMessageRef::new("msg:auth-fanout-resolve")
            .expect("accepted message ref"), // safety: static test ref is valid.
        submitted_run_id: blocked_run_id,
    };
    let auth_envelope = auth_resolution_allowed_envelope("callback:test-fanout");
    observer.observe_ack(auth_envelope, auth_ack).await;

    // Later Resumed/Completed facts independently drive the final reply.
    harness
        .coordinator
        .resume_blocked_run_to_running()
        .await
        .expect("resume auth-blocked run");
    harness
        .coordinator
        .complete_active_run("auth completed and finished")
        .await
        .expect("complete resumed auth run");

    // Total: one auth prompt, the resumed-run thinking indicator, and one
    // final reply. The auth-resolution acknowledgement itself adds no owner.
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        3,
        "expected exactly 3 messages: auth prompt + resumed thinking indicator + final reply, not {} (duplicate final reply was posted without the fix)",
        messages.len()
    );
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Authentication required")),
        "messages[0] must be the auth prompt"
    );
    assert_eq!(
        messages[1]["text"], "Ironclaw is thinking...",
        "messages[1] must be the resumed-run thinking indicator"
    );
    assert_eq!(
        messages[2]["text"], "auth completed and finished",
        "messages[2] must be the final reply"
    );
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

#[tokio::test]
async fn slack_approval_then_auth_resume_completes_without_second_approval() {
    let auth_provider = Arc::new(FakeAuthChallengeProvider::default());
    let auth_challenges: Arc<dyn AuthChallengeProvider> = auth_provider.clone();
    let harness =
        build_harness_with_auth_challenges(TurnMode::BlockApprovalThenAuth, Some(auth_challenges))
            .await;

    // The inbound run first commits BlockedApproval.
    let first = harness.post_event(DM_APPROVAL_THEN_AUTH_BLOCK).await;
    assert_eq!(first.status(), StatusCode::OK);

    // Wait for its approval prompt.
    for _ in 0..200 {
        if harness.slack_messages().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        1,
        "expected exactly one approval prompt; got {}: {:?}",
        messages.len(),
        messages
    );
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Approval needed")),
        "first message must be the approval prompt; got {:?}",
        messages[0]["text"]
    );

    // Post the approve event through the real inbound path.
    // RecordingApprovalInteractionService::resolve sees BlockApprovalThenAuth mode
    // and transitions the run to BlockedAuth instead of completing it.
    let approve = harness
        .post_event(dm_message("Ev-approval-then-auth-approve", "approve"))
        .await;
    assert_eq!(approve.status(), StatusCode::OK);
    // The committed BlockedAuth state independently owns its auth prompt.
    for _ in 0..200 {
        if harness.slack_messages().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        2,
        "expected approval prompt + auth prompt; got {}: {:?}",
        messages.len(),
        messages
    );
    assert!(
        messages[1]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Authentication required")),
        "second message must be the auth prompt; got {:?}",
        messages[1]["text"]
    );

    // Advance BlockedAuth -> Completed in one committed mutation.
    harness
        .coordinator
        .complete_blocked_run("approved then authed and finished")
        .await
        .expect("complete auth-blocked run");

    // Wait for the Completed event's final reply.
    for _ in 0..200 {
        if harness.slack_messages().len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    harness.drain().await;

    let messages = harness.slack_messages();
    assert_eq!(
        messages.len(),
        3,
        "expected 3 messages: approval prompt + auth prompt + final reply, got {}: {:?}",
        messages.len(),
        messages
    );
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Approval needed")),
        "messages[0] must be the approval prompt"
    );
    assert!(
        messages[1]["text"]
            .as_str()
            .is_some_and(|t| t.contains("Authentication required")),
        "messages[1] must be the auth prompt"
    );
    assert_eq!(
        messages[2]["text"], "approved then authed and finished",
        "messages[2] must be the final reply, delivered exactly once"
    );

    let deletes = harness.slack_deletes();
    assert_eq!(
        deletes.len(),
        2,
        "expected 2 deletes: approval and auth prompts deleted after the final reply, got {}",
        deletes.len()
    );

    // Exactly 1 approval-service request: the approve event was routed through
    // RecordingApprovalInteractionService::resolve (the real caller), not via
    // the coordinator backdoor. Satisfies the Test-Through-the-Caller rule.
    let approvals = harness.approvals.requests();
    assert_eq!(
        approvals.len(),
        1,
        "expected 1 approval-service request (routed through the caller, not via backdoor), got {}",
        approvals.len()
    );

    // Exactly 1 turn submitted (no re-submission).
    let submitted = harness.coordinator.submitted_turn_count();
    assert_eq!(
        submitted, 1,
        "expected exactly 1 submitted turn, got {}",
        submitted
    );

    // FakeAuthChallengeProvider must have been called exactly once (for the auth prompt).
    auth_provider.assert_single_call();
}

// ─── Generic outbound-delivery targets + generic triggered hook (P6 c-rest) ─

use crate::extension_host::channel_dm_targets::{ChannelDmTargetStore, dm_target_payload};
use crate::extension_host::channel_outbound_targets::{
    ChannelOutboundTargetIdentity, GenericChannelOutboundTargetDeps,
    GenericChannelOutboundTargetProvider,
};
use crate::extension_host::channel_triggered_delivery::GenericTriggeredRunDeliveryHook;
use crate::outbound::OutboundDeliveryTargetProvider;
use ironclaw_outbound::{OutboundDeliveryTargetScope, TriggeredRunDeliveryStore};

/// The retired Slack setup surface's installation id — DIFFERENT from the
/// durable extension installation id (`INSTALLATION`) the active snapshot
/// carries. Stored beta preferences embed this id in their binding refs.
const RETIRED_INSTALLATION: &str = "retired-setup-install";
/// A shared channel routed to the operator through `slack_subject_routes`.
const ROUTED_CHANNEL: &str = "C777";

fn generic_dm_target_store() -> Arc<ChannelDmTargetStore> {
    Arc::new(ChannelDmTargetStore::new(
        Arc::new(InMemoryBackend::new()),
        TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
        UserId::new(USER).expect("user"),       // safety: static test user id is valid.
    ))
}

fn generic_outbound_target_provider(
    harness: &Harness,
    dm_targets: Arc<ChannelDmTargetStore>,
) -> GenericChannelOutboundTargetProvider {
    GenericChannelOutboundTargetProvider::new(GenericChannelOutboundTargetDeps {
        watch: harness.assembly.snapshot_watch(),
        assembly: Arc::clone(&harness.assembly),
        admin_configuration_resolver: Arc::clone(&harness.admin_configuration_resolver),
        installation_store: Arc::clone(&harness.installation_store)
            as Arc<dyn ExtensionInstallationStorePort>,
        dm_targets,
        identity: ChannelOutboundTargetIdentity {
            tenant_id: TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            agent_id: AgentId::new(AGENT).expect("agent"), // safety: static test agent id is valid.
            project_id: Some(ProjectId::new(PROJECT).expect("project")), // safety: static test project id is valid.
        },
    })
}

fn operator_caller() -> OutboundDeliveryTargetScope {
    OutboundDeliveryTargetScope::new(
        TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
        UserId::new(USER).expect("user"),       // safety: static test user id is valid.
    )
}

/// Save the administrator values the generic target provider reads:
/// the workspace claim (space id) and one explicit subject route assigning
/// `ROUTED_CHANNEL` to the operator.
async fn save_outbound_target_config(harness: &Harness) {
    harness
        .admin_configuration_resolver
        .configure_admin_group_for_test(
            "extension.slack",
            vec![
                ("slack_team_id".to_string(), TEAM.to_string()),
                (
                    "slack_subject_routes".to_string(),
                    format!(r#"{{"{ROUTED_CHANNEL}":"{USER}"}}"#),
                ),
            ],
        )
        .await
        .expect("save outbound target config"); // safety: manifest declares the handles.
}

/// The generic provider lists the operator's routed shared channel (from
/// `slack_subject_routes`) and their provisioned personal DM (from the
/// generic DM-target store) — no lane-owned state anywhere. The real Slack
/// `conversations.open` response supplies the DM conversation id but not the
/// stable workspace id, so the provider must complete that generic target
/// shape from the administrator-configured channel context.
#[tokio::test]
async fn generic_outbound_targets_list_from_admin_configuration_and_generic_dm_store() {
    let harness = build_harness(TurnMode::Running).await;
    save_outbound_target_config(&harness).await;
    let dm_targets = generic_dm_target_store();
    dm_targets
        .upsert(
            ADAPTER,
            &UserId::new(USER).expect("user"), // safety: static test user id is valid.
            SLACK_USER.to_string(),
            dm_target_payload(None, CHANNEL),
        )
        .await
        .expect("provision DM target");
    let provider = generic_outbound_target_provider(&harness, dm_targets);
    let codec = SlackPreferenceTargetCodec;

    let listed = provider
        .list_outbound_delivery_targets(&operator_caller())
        .await
        .expect("target list");
    assert_eq!(listed.len(), 2, "one shared + one DM target: {listed:?}");

    let shared = listed
        .iter()
        .find(|entry| entry.summary.target_id.as_str().contains("shared-channel"))
        .expect("shared-channel target listed");
    assert_eq!(
        shared.summary.target_id.as_str(),
        format!("slack:shared-channel:{TEAM}:{ROUTED_CHANNEL}"),
        "generic ids keep the retired lane's shape"
    );
    let shared_conversation = codec
        .conversation_for_target(&outbound_reply_target(shared))
        .expect("shared binding ref decodes");
    assert_eq!(shared_conversation.conversation_id(), ROUTED_CHANNEL);
    assert_eq!(shared_conversation.space_id(), Some(TEAM));
    assert!(!codec.is_personal_direct_message(&outbound_reply_target(shared)));

    let dm = listed
        .iter()
        .find(|entry| entry.summary.target_id.as_str().contains("personal-dm"))
        .expect("personal-DM target listed");
    assert_eq!(
        dm.summary.target_id.as_str(),
        format!("slack:personal-dm:{TEAM}:{USER}")
    );
    assert!(codec.is_personal_direct_message(&outbound_reply_target(dm)));
    assert_eq!(
        codec.direct_message_actor_for_target(&outbound_reply_target(dm)),
        Some(SLACK_USER.to_string()),
        "the encoded DM ref carries the provisioned actor"
    );
    // The encoded refs carry the DURABLE installation id from the snapshot.
    assert!(
        outbound_reply_target(dm).as_str().contains(&format!(
            "installation:{}:{INSTALLATION};",
            INSTALLATION.len()
        )),
        "DM ref must embed the durable installation id: {}",
        outbound_reply_target(dm).as_str()
    );

    // resolve-by-id round-trips for the owner…
    for entry in &listed {
        let resolved = provider
            .resolve_outbound_delivery_target(&operator_caller(), &entry.summary.target_id)
            .await
            .expect("resolve succeeds")
            .expect("owner resolves the listed target");
        assert_eq!(resolved.summary.target_id, entry.summary.target_id);
    }
    // …and fails closed for foreign callers.
    let foreign_tenant = OutboundDeliveryTargetScope::new(
        TenantId::new("tenant:other").expect("tenant"), // safety: static test tenant id is valid.
        UserId::new(USER).expect("user"),               // safety: static test user id is valid.
    );
    assert!(
        provider
            .list_outbound_delivery_targets(&foreign_tenant)
            .await
            .expect("list succeeds")
            .is_empty(),
        "cross-tenant caller sees no targets"
    );
    let other_user = OutboundDeliveryTargetScope::new(
        TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
        UserId::new("user:slack-bob").expect("user"), // safety: static test user id is valid.
    );
    assert!(
        provider
            .list_outbound_delivery_targets(&other_user)
            .await
            .expect("list succeeds")
            .is_empty(),
        "another user sees neither the operator's routed channel nor their DM"
    );
    for entry in &listed {
        assert!(
            provider
                .resolve_outbound_delivery_target(&other_user, &entry.summary.target_id)
                .await
                .expect("resolve succeeds")
                .is_none(),
            "another user must not resolve the operator's target {}",
            entry.summary.target_id.as_str()
        );
    }
}

/// Removing one caller from a shared installation revokes that caller's
/// outbound target authority immediately, without tearing down the active
/// deployment, administrator configuration, or another member's targets.
/// This is the production separation between tenant configuration/runtime
/// and per-user extension membership.
#[tokio::test]
async fn generic_outbound_targets_require_current_non_last_member_authority() {
    let harness = build_harness(TurnMode::Running).await;
    save_outbound_target_config(&harness).await;
    let dm_targets = generic_dm_target_store();
    let alice = UserId::new(USER).expect("Alice user");
    let bob = UserId::new("user:slack-bob").expect("Bob user");
    dm_targets
        .upsert(
            ADAPTER,
            &alice,
            SLACK_USER.to_string(),
            dm_target_payload(Some(TEAM), CHANNEL),
        )
        .await
        .expect("provision Alice DM target");
    dm_targets
        .upsert(
            ADAPTER,
            &bob,
            "U456".to_string(),
            dm_target_payload(Some(TEAM), "D456"),
        )
        .await
        .expect("provision Bob DM target");
    let provider = generic_outbound_target_provider(&harness, dm_targets);
    let bob_caller =
        OutboundDeliveryTargetScope::new(TenantId::new(TENANT).expect("tenant"), bob.clone());

    let alice_targets = provider
        .list_outbound_delivery_targets(&operator_caller())
        .await
        .expect("list Alice targets");
    let bob_targets = provider
        .list_outbound_delivery_targets(&bob_caller)
        .await
        .expect("list Bob targets");
    assert_eq!(alice_targets.len(), 2, "Alice initially owns shared + DM");
    assert_eq!(bob_targets.len(), 1, "Bob initially owns only his DM");
    assert!(
        bob_targets[0]
            .summary
            .target_id
            .as_str()
            .contains(&format!(":{TEAM}:")),
        "Bob's target still uses tenant administrator configuration"
    );

    let installation_id = ExtensionInstallationId::new(INSTALLATION).expect("installation id");
    let installation = harness
        .installation_store
        .get_installation(&installation_id)
        .await
        .expect("load installation")
        .expect("shared Slack installation exists");
    harness
        .installation_store
        .upsert_installation(installation.with_owner(InstallationOwner::user(bob.clone())))
        .await
        .expect("persist non-last-member removal");

    assert!(
        harness
            .assembly
            .snapshot_watch()
            .current()
            .extension(ADAPTER)
            .is_some(),
        "removing Alice must not tear down Bob's shared active runtime"
    );
    assert_eq!(
        harness
            .admin_configuration_resolver
            .non_secret_value(
                &ExtensionId::new(ADAPTER).expect("extension id"),
                "slack_team_id",
            )
            .await
            .expect("read admin configuration")
            .as_deref(),
        Some(TEAM),
        "personal removal must not alter tenant administrator configuration"
    );

    assert!(
        provider
            .list_outbound_delivery_targets(&operator_caller())
            .await
            .expect("list removed caller targets")
            .is_empty(),
        "the operator's admin privilege cannot replace removed personal membership"
    );
    for target in &alice_targets {
        assert!(
            provider
                .resolve_outbound_delivery_target(&operator_caller(), &target.summary.target_id,)
                .await
                .expect("resolve removed caller target")
                .is_none(),
            "removed Alice must not resolve target id {}",
            target.summary.target_id.as_str()
        );
        assert!(
            provider
                .resolve_reply_target_binding(&operator_caller(), &outbound_reply_target(target),)
                .await
                .expect("resolve removed caller binding")
                .is_none(),
            "removed Alice must not resolve a previously sealed binding"
        );
    }

    let bob_after_removal = provider
        .list_outbound_delivery_targets(&bob_caller)
        .await
        .expect("list remaining member targets");
    assert_eq!(
        bob_after_removal
            .iter()
            .map(|entry| entry.summary.target_id.as_str())
            .collect::<Vec<_>>(),
        bob_targets
            .iter()
            .map(|entry| entry.summary.target_id.as_str())
            .collect::<Vec<_>>(),
        "Bob keeps his independent target after Alice leaves"
    );
    assert!(
        provider
            .resolve_outbound_delivery_target(&bob_caller, &bob_targets[0].summary.target_id,)
            .await
            .expect("resolve Bob target")
            .is_some(),
        "remaining member still resolves by target id"
    );
    assert!(
        provider
            .resolve_reply_target_binding(&bob_caller, &outbound_reply_target(&bob_targets[0]),)
            .await
            .expect("resolve Bob binding")
            .is_some(),
        "remaining member still resolves by sealed binding"
    );
}

/// REGRESSION (migration tolerance): stored beta preferences embed the
/// RETIRED setup installation id in their binding refs. Resolution must
/// tolerate both ids — ownership is proven against caller-scoped generic
/// state, never against the ref's installation segment — and each resolve
/// returns a freshly encoded ref carrying the DURABLE installation id.
#[tokio::test]
async fn generic_outbound_targets_tolerate_retired_installation_id_binding_refs() {
    let harness = build_harness(TurnMode::Running).await;
    save_outbound_target_config(&harness).await;
    let dm_targets = generic_dm_target_store();
    dm_targets
        .upsert(
            ADAPTER,
            &UserId::new(USER).expect("user"), // safety: static test user id is valid.
            SLACK_USER.to_string(),
            dm_target_payload(Some(TEAM), CHANNEL),
        )
        .await
        .expect("provision DM target");
    let provider = generic_outbound_target_provider(&harness, dm_targets);

    let retired_installation =
        AdapterInstallationId::new(RETIRED_INSTALLATION).expect("installation"); // safety: static id is valid.
    let agent = AgentId::new(AGENT).expect("agent"); // safety: static test agent id is valid.
    let project = ProjectId::new(PROJECT).expect("project"); // safety: static test project id is valid.
    let durable_segment = format!("installation:{}:{INSTALLATION};", INSTALLATION.len());

    // Shared-channel preference saved under the retired setup id.
    let retired_shared = ironclaw_slack_extension::slack_shared_channel_reply_target_binding_ref(
        &retired_installation,
        &agent,
        Some(&project),
        TEAM,
        ROUTED_CHANNEL,
    )
    .expect("retired shared ref builds");
    let resolved_shared = provider
        .resolve_reply_target_binding(&operator_caller(), &retired_shared)
        .await
        .expect("resolve succeeds")
        .expect("retired-id shared preference still resolves");
    assert!(
        outbound_reply_target(&resolved_shared)
            .as_str()
            .contains(&durable_segment),
        "re-resolved ref carries the durable installation id: {}",
        outbound_reply_target(&resolved_shared).as_str()
    );

    // Personal-DM preference saved under the retired setup id.
    let retired_dm = ironclaw_slack_extension::slack_personal_dm_reply_target_binding_ref(
        &retired_installation,
        &agent,
        Some(&project),
        TEAM,
        CHANNEL,
        SLACK_USER,
    )
    .expect("retired DM ref builds");
    let resolved_dm = provider
        .resolve_reply_target_binding(&operator_caller(), &retired_dm)
        .await
        .expect("resolve succeeds")
        .expect("retired-id DM preference still resolves");
    assert!(
        outbound_reply_target(&resolved_dm)
            .as_str()
            .contains(&durable_segment),
        "re-resolved DM ref carries the durable installation id: {}",
        outbound_reply_target(&resolved_dm).as_str()
    );

    // Fail-closed arms: a tampered actor never resolves; an unrouted
    // conversation never resolves (regardless of which id the ref carries).
    let tampered_actor = ironclaw_slack_extension::slack_personal_dm_reply_target_binding_ref(
        &retired_installation,
        &agent,
        Some(&project),
        TEAM,
        CHANNEL,
        "U_EVIL",
    )
    .expect("tampered DM ref builds");
    assert!(
        provider
            .resolve_reply_target_binding(&operator_caller(), &tampered_actor)
            .await
            .expect("resolve succeeds")
            .is_none(),
        "a DM ref with a foreign actor must not resolve"
    );
    let unrouted = ironclaw_slack_extension::slack_shared_channel_reply_target_binding_ref(
        &retired_installation,
        &agent,
        Some(&project),
        TEAM,
        "C999",
    )
    .expect("unrouted shared ref builds");
    assert!(
        provider
            .resolve_reply_target_binding(&operator_caller(), &unrouted)
            .await
            .expect("resolve succeeds")
            .is_none(),
        "an unrouted shared conversation must not resolve"
    );
}

/// The generic triggered hook routes a settled fire to the owning
/// extension's driver: the creator's stored preference decodes through the
/// slack codec registered in the assembly extras, the driver is built from
/// the assembly's OWN delivery services, and the approval prompt lands on
/// the harness egress with the delivered gate route recorded.
#[tokio::test]
async fn generic_triggered_hook_routes_fire_to_the_owning_extension_driver() {
    let (harness, _approvals) = build_harness_for_delivered_route_tests().await;

    // A blocked run the coordinator knows about (the hook's event consumer
    // reads the SAME canonical coordinator state the assembly wires).
    let block_response = harness.post_event(DM_BLOCK).await;
    assert_eq!(block_response.status(), StatusCode::OK);
    harness.drain().await;
    let blocked_run_id = harness
        .coordinator
        .blocked_run_id()
        .expect("run must be blocked after DM_BLOCK"); // safety: E2E test assertion.
    harness
        .coordinator
        .set_run_origin(blocked_run_id, TurnOriginKind::ScheduledTrigger)
        .expect("triggered run fixture has product context"); // safety: fixture creates the run and its product context.

    let tenant = TenantId::new(TENANT).expect("tenant"); // safety: static test tenant id is valid.
    let user = UserId::new(USER).expect("user"); // safety: static test user id is valid.

    // Seed the creator's personal preference on the SAME store the
    // assembly's delivery deps read.
    let dm_target = dm_reply_target_binding_ref();
    harness
        .outbound
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(tenant.clone(), user.clone()),
                final_reply_target: Some(dm_target.clone()),
                progress_target: None,
                approval_prompt_target: Some(dm_target.clone()),
                auth_prompt_target: None,
                default_modality: None,
                updated_at: chrono::Utc::now(),
                updated_by: user.clone(),
            },
            expected_version: None,
        })
        .await
        .expect("seed personal preference"); // safety: in-memory store should not fail.

    let delivery_store = Arc::new(in_memory_backed_outbound_state_store());
    let hook = GenericTriggeredRunDeliveryHook::new(
        Arc::clone(&harness.assembly),
        Arc::clone(&delivery_store) as Arc<dyn TriggeredRunDeliveryStore>,
        harness.outbound.clone() as Arc<dyn CommunicationPreferenceRepository>,
        current_target_resolver(
            &harness.assembly,
            Arc::new(crate::outbound::MutableOutboundDeliveryTargetRegistry::default()),
        ),
        Arc::clone(&harness.event_router),
    );

    let fire = TriggerFire {
        identity: TriggerFireIdentity::new(tenant.clone(), TriggerId::new(), chrono::Utc::now()),
        creator_user_id: user.clone(),
        agent_id: None,
        project_id: None,
        prompt: "generic triggered delivery".to_string(),
        delivery_target: None,
    };
    use crate::automation::trigger_poller::PostSubmitDeliveryHook as _;
    hook.on_trigger_submitted(fire, blocked_run_id, foreign_run_scope())
        .await;

    // The routed slack driver posted the approval prompt to the creator's DM
    // through the assembly's delivery coordinator (harness egress).
    let approval_prompts = wait_for_approval_prompt_messages(&harness.egress, GATE).await;
    assert_eq!(
        approval_prompts.len(),
        1,
        "exactly one approval-prompt chat.postMessage: {approval_prompts:?}"
    );
    assert_eq!(approval_prompts[0]["channel"], CHANNEL);

    // …and auto-recorded the delivered gate route into the assembly's store.
    let route = wait_for_gate_route_matching(
        harness.route_store.as_ref(),
        &tenant,
        &user,
        GATE,
        |record| record.run_id == blocked_run_id,
    )
    .await;
    assert!(
        route
            .delivered_conversation_fingerprints
            .contains(&dm_conversation_fingerprint()),
        "driver route carries the DM conversation fingerprint: {:?}",
        route.delivered_conversation_fingerprints
    );

    // Fail-closed routing: a stored preference no registered codec decodes
    // records a Failed outcome instead of guessing a channel.
    harness
        .outbound
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(tenant.clone(), user.clone()),
                final_reply_target: Some(
                    ReplyTargetBindingRef::new("reply:adapter:5:other;rest").expect("ref"), // safety: static test ref is valid.
                ),
                progress_target: None,
                approval_prompt_target: None,
                auth_prompt_target: None,
                default_modality: None,
                updated_at: chrono::Utc::now(),
                updated_by: user.clone(),
            },
            expected_version: Some(ironclaw_outbound::CommunicationPreferenceVersion::from_raw(
                1,
            )),
        })
        .await
        .expect("overwrite preference with a foreign-vendor target");
    let foreign_fire = TriggerFire {
        identity: TriggerFireIdentity::new(tenant.clone(), TriggerId::new(), chrono::Utc::now()),
        creator_user_id: user.clone(),
        agent_id: None,
        project_id: None,
        prompt: "unroutable triggered delivery".to_string(),
        delivery_target: None,
    };
    let unroutable_run_id = TurnRunId::new();
    hook.on_trigger_submitted(foreign_fire, unroutable_run_id, foreign_run_scope())
        .await;
    let record = delivery_store
        .load_triggered_run_delivery(unroutable_run_id)
        .await
        .expect("load outcome")
        .expect("unroutable fire records an outcome");
    assert_eq!(
        record.outcome,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Failed,
        "an undecodable stored target fails closed"
    );
}

/// A trigger's own target id is resolved at fire time through the same
/// creator-scoped registry used during creation, then delivered without a
/// user-global preference. This is the whole composition path used by
/// Telegram, Slack, and future manifest-driven channel providers.
#[tokio::test]
async fn generic_triggered_hook_honors_per_trigger_target_without_global_default() {
    let harness = build_harness(TurnMode::Running).await;
    save_outbound_target_config(&harness).await;
    let dm_targets = generic_dm_target_store();
    let user = UserId::new(USER).expect("user"); // safety: static test user id is valid.
    dm_targets
        .upsert(
            ADAPTER,
            &user,
            SLACK_USER.to_string(),
            dm_target_payload(Some(TEAM), CHANNEL),
        )
        .await
        .expect("provision DM target");
    let provider = Arc::new(generic_outbound_target_provider(&harness, dm_targets));
    let listed = provider
        .list_outbound_delivery_targets(&operator_caller())
        .await
        .expect("list targets");
    let target_id = listed
        .iter()
        .find(|entry| entry.summary.target_id.as_str().contains("personal-dm"))
        .expect("personal target")
        .summary
        .target_id
        .as_str()
        .to_string();
    let registry = Arc::new(crate::outbound::MutableOutboundDeliveryTargetRegistry::default());
    registry
        .register_provider(
            "channel",
            provider as Arc<dyn OutboundDeliveryTargetProvider>,
        )
        .expect("register target provider");

    let scope = foreign_run_scope();
    harness.ensure_scope_thread(&scope).await;
    let run_id = TurnRunId::new();
    harness
        .coordinator
        .complete_run(
            scope.clone(),
            TurnActor::new(user.clone()),
            run_id,
            "scheduled result",
            TurnOriginKind::ScheduledTrigger,
        )
        .await
        .expect("seed completed trigger run");

    let delivery_store = Arc::new(in_memory_backed_outbound_state_store());
    let hook = GenericTriggeredRunDeliveryHook::new(
        Arc::clone(&harness.assembly),
        Arc::clone(&delivery_store) as Arc<dyn TriggeredRunDeliveryStore>,
        harness.outbound.clone() as Arc<dyn CommunicationPreferenceRepository>,
        current_target_resolver(&harness.assembly, registry),
        Arc::clone(&harness.event_router),
    );
    let fire = TriggerFire {
        identity: TriggerFireIdentity::new(
            TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            TriggerId::new(),
            chrono::Utc::now(),
        ),
        creator_user_id: user,
        agent_id: Some(AgentId::new(AGENT).expect("agent")), // safety: static test agent id is valid.
        project_id: None,
        prompt: "scheduled result to this DM".to_string(),
        delivery_target: Some(
            ironclaw_triggers::TriggerDeliveryTargetId::new(target_id).expect("target id"),
        ),
    };
    use crate::automation::trigger_poller::PostSubmitDeliveryHook as _;
    hook.on_trigger_submitted(fire, run_id, scope).await;

    for _ in 0..200 {
        if delivery_store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load outcome")
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let record = delivery_store
        .load_triggered_run_delivery(run_id)
        .await
        .expect("load outcome")
        .expect("delivery outcome");
    assert_eq!(
        record.outcome,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Delivered
    );
    let messages = harness.slack_messages();
    assert_eq!(messages.len(), 1, "one per-trigger result: {messages:?}");
    assert_eq!(messages[0]["channel"], CHANNEL);
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("scheduled result")),
        "final result reaches the selected target: {messages:?}"
    );
}

/// WebApp is a host-owned final-reply destination, so a trigger that seals it
/// must not be sent through (or failed by) the external channel delivery hook.
#[tokio::test]
async fn generic_triggered_hook_leaves_web_app_target_in_run_history() {
    let harness = build_harness(TurnMode::Running).await;
    let delivery_store = Arc::new(in_memory_backed_outbound_state_store());
    let registry = Arc::new(crate::outbound::MutableOutboundDeliveryTargetRegistry::default());
    registry
        .register_provider(
            ironclaw_outbound::WEB_APP_OUTBOUND_DELIVERY_TARGET_ID,
            Arc::new(
                ironclaw_outbound::HostOwnedOutboundDeliveryTargetProvider::new(
                    ironclaw_outbound::OutboundDeliveryTargetSummary::new(
                        ironclaw_outbound::OutboundDeliveryTargetId::new(
                            ironclaw_outbound::WEB_APP_OUTBOUND_DELIVERY_TARGET_ID,
                        )
                        .expect("host WebApp target id"),
                        "web_app",
                        "Web app only",
                        Some(
                            "Store the final answer in run history without external delivery."
                                .to_string(),
                        ),
                    )
                    .expect("host WebApp target summary"),
                    ironclaw_outbound::DeliveryTargetCapabilities {
                        final_replies: true,
                        progress: false,
                        gate_prompts: false,
                        auth_prompts: false,
                        modalities: Vec::new(),
                    },
                    ironclaw_outbound::RunFinalReplyDestination::WebApp,
                ),
            ),
        )
        .expect("register host WebApp target provider");
    let current_targets = Arc::new(
        crate::extension_host::channel_outbound_targets::ComposedCurrentDeliveryTargetResolver::new(
            Arc::clone(&registry),
        ),
    );
    let hook = GenericTriggeredRunDeliveryHook::new(
        Arc::clone(&harness.assembly),
        Arc::clone(&delivery_store) as Arc<dyn TriggeredRunDeliveryStore>,
        harness.outbound.clone() as Arc<dyn CommunicationPreferenceRepository>,
        current_targets as Arc<dyn CurrentDeliveryTargetResolver>,
        Arc::clone(&harness.event_router),
    );
    let run_id = TurnRunId::new();
    let fire = TriggerFire {
        identity: TriggerFireIdentity::new(
            TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            TriggerId::new(),
            chrono::Utc::now(),
        ),
        creator_user_id: UserId::new(USER).expect("user"), // safety: static test user id is valid.
        agent_id: Some(AgentId::new(AGENT).expect("agent")), // safety: static test agent id is valid.
        project_id: None,
        prompt: "scheduled result in WebApp".to_string(),
        delivery_target: Some(
            ironclaw_triggers::TriggerDeliveryTargetId::new(
                ironclaw_product::WEB_APP_OUTBOUND_DELIVERY_TARGET_ID,
            )
            .expect("host WebApp target"),
        ),
    };

    use crate::automation::trigger_poller::PostSubmitDeliveryHook as _;
    hook.on_trigger_submitted(fire, run_id, foreign_run_scope())
        .await;

    assert!(
        delivery_store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load outcome")
            .is_none(),
        "external channel delivery does not claim or fail a WebApp-only run"
    );
    assert!(
        harness.slack_messages().is_empty(),
        "WebApp-only trigger must not emit through an external channel"
    );
}

async fn completed_trigger_run_for_user(
    harness: &Harness,
    user: &UserId,
    thread_id: &str,
    result: &str,
) -> (TurnScope, TurnRunId) {
    let scope = TurnScope::new_with_owner(
        TenantId::new(TENANT).expect("tenant"), // safety: cfg(test)-only static fixture is valid.
        Some(AgentId::new(AGENT).expect("agent")), // safety: cfg(test)-only static fixture is valid.
        Some(ProjectId::new(PROJECT).expect("project")), // safety: cfg(test)-only static fixture is valid.
        ThreadId::new(thread_id).expect("thread"), // safety: cfg(test)-only caller supplies a valid fixture id.
        Some(user.clone()),
    );
    harness.ensure_scope_thread(&scope).await;
    let run_id = TurnRunId::new();
    harness
        .coordinator
        .complete_run(
            scope.clone(),
            TurnActor::new(user.clone()),
            run_id,
            result,
            TurnOriginKind::ScheduledTrigger,
        )
        .await
        .expect("seed completed trigger run"); // safety: cfg(test)-only fixture setup must succeed.
    (scope, run_id)
}

async fn wait_for_triggered_delivery_outcome(
    store: &dyn TriggeredRunDeliveryStore,
    run_id: TurnRunId,
) -> ironclaw_outbound::TriggeredRunDeliveryOutcomeKind {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let loaded = store.load_triggered_run_delivery(run_id).await;
            let record = loaded.expect("load outcome"); // safety: cfg(test)-only in-memory store must remain readable.
            if let Some(record) = record {
                return record.outcome;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("terminal outcome") // safety: cfg(test)-only bounded wait asserts fixture completion.
}

/// A sealed target is only a locator, never permanent delivery authority.
/// The production registry/provider must revalidate the creator, active
/// extension snapshot, and pairing-provisioned DM record immediately before
/// egress. Revocation, removal, and reinstall-without-repairing all fail
/// closed; no single-channel/default fallback may send to the stale wire.
#[tokio::test]
async fn generic_triggered_delivery_revalidates_current_authority_before_egress() {
    let harness = build_harness(TurnMode::Running).await;
    save_outbound_target_config(&harness).await;
    let dm_targets = generic_dm_target_store();
    let alice = UserId::new(USER).expect("Alice user");
    let bob = UserId::new("user:slack-bob").expect("Bob user");
    dm_targets
        .upsert(
            ADAPTER,
            &alice,
            SLACK_USER.to_string(),
            dm_target_payload(Some(TEAM), CHANNEL),
        )
        .await
        .expect("provision Alice DM target");
    let provider = Arc::new(generic_outbound_target_provider(
        &harness,
        Arc::clone(&dm_targets),
    ));
    let alice_target = provider
        .list_outbound_delivery_targets(&operator_caller())
        .await
        .expect("list Alice targets")
        .into_iter()
        .find(|entry| entry.summary.target_id.as_str().contains("personal-dm"))
        .expect("Alice personal target");
    let registry = Arc::new(crate::outbound::MutableOutboundDeliveryTargetRegistry::default());
    registry
        .register_provider(
            "channel",
            provider as Arc<dyn OutboundDeliveryTargetProvider>,
        )
        .expect("register production target provider");
    let delivery_store = Arc::new(in_memory_backed_outbound_state_store());
    let hook = GenericTriggeredRunDeliveryHook::new(
        Arc::clone(&harness.assembly),
        Arc::clone(&delivery_store) as Arc<dyn TriggeredRunDeliveryStore>,
        harness.outbound.clone() as Arc<dyn CommunicationPreferenceRepository>,
        current_target_resolver(&harness.assembly, Arc::clone(&registry)),
        Arc::clone(&harness.event_router),
    );

    // Store the exact binding emitted by the production provider, then revoke
    // its pairing-provisioned DM record. The driver must reject the stale
    // preference on its final current-authority lookup, not fall back to the
    // only active channel.
    harness
        .outbound
        .write_communication_preference(WriteCommunicationPreferenceRequest {
            record: CommunicationPreferenceRecord {
                scope: DeliveryDefaultScope::personal(
                    TenantId::new(TENANT).expect("tenant"),
                    alice.clone(),
                ),
                final_reply_target: Some(outbound_reply_target(&alice_target)),
                progress_target: None,
                approval_prompt_target: None,
                auth_prompt_target: None,
                default_modality: None,
                updated_at: chrono::Utc::now(),
                updated_by: alice.clone(),
            },
            expected_version: None,
        })
        .await
        .expect("store Alice target preference");
    dm_targets
        .delete(ADAPTER, &alice)
        .await
        .expect("revoke Alice DM target");
    let (revoked_scope, revoked_run) = completed_trigger_run_for_user(
        &harness,
        &alice,
        "thread:revoked-target",
        "must not reach revoked target",
    )
    .await;
    use crate::automation::trigger_poller::PostSubmitDeliveryHook as _;
    hook.on_trigger_submitted(
        TriggerFire {
            identity: TriggerFireIdentity::new(
                TenantId::new(TENANT).expect("tenant"),
                TriggerId::new(),
                chrono::Utc::now(),
            ),
            creator_user_id: alice.clone(),
            agent_id: Some(AgentId::new(AGENT).expect("agent")),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            prompt: "revoked target".to_string(),
            delivery_target: None,
        },
        revoked_run,
        revoked_scope,
    )
    .await;
    assert_eq!(
        wait_for_triggered_delivery_outcome(delivery_store.as_ref(), revoked_run).await,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Denied,
        "a target revoked after registration is an authority denial"
    );
    assert!(
        harness.slack_messages().is_empty(),
        "revoked stored target must not fall back to Slack wire egress"
    );

    // Restore Alice's authority, then present her opaque target as Bob. Both
    // provider-side ownership and the aggregate registry must reject it.
    dm_targets
        .upsert(
            ADAPTER,
            &alice,
            SLACK_USER.to_string(),
            dm_target_payload(Some(TEAM), CHANNEL),
        )
        .await
        .expect("restore Alice DM target");
    let (foreign_scope, foreign_run) = completed_trigger_run_for_user(
        &harness,
        &bob,
        "thread:foreign-user-target",
        "must not reach Alice target",
    )
    .await;
    hook.on_trigger_submitted(
        TriggerFire {
            identity: TriggerFireIdentity::new(
                TenantId::new(TENANT).expect("tenant"),
                TriggerId::new(),
                chrono::Utc::now(),
            ),
            creator_user_id: bob,
            agent_id: Some(AgentId::new(AGENT).expect("agent")),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            prompt: "foreign target".to_string(),
            delivery_target: Some(
                ironclaw_triggers::TriggerDeliveryTargetId::new(
                    alice_target.summary.target_id.as_str().to_string(),
                )
                .expect("target id"),
            ),
        },
        foreign_run,
        foreign_scope,
    )
    .await;
    assert_eq!(
        wait_for_triggered_delivery_outcome(delivery_store.as_ref(), foreign_run).await,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Failed
    );
    assert!(
        harness.slack_messages().is_empty(),
        "Bob must not deliver through Alice's target"
    );

    // Removing the extension invalidates the provider context even while a
    // stale DM record and target id still exist.
    harness
        ._host
        .deactivate(ADAPTER)
        .await
        .expect("deactivate channel extension");
    let (removed_scope, removed_run) = completed_trigger_run_for_user(
        &harness,
        &alice,
        "thread:removed-extension-target",
        "must not reach removed extension",
    )
    .await;
    hook.on_trigger_submitted(
        TriggerFire {
            identity: TriggerFireIdentity::new(
                TenantId::new(TENANT).expect("tenant"),
                TriggerId::new(),
                chrono::Utc::now(),
            ),
            creator_user_id: alice.clone(),
            agent_id: Some(AgentId::new(AGENT).expect("agent")),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            prompt: "removed extension".to_string(),
            delivery_target: Some(
                ironclaw_triggers::TriggerDeliveryTargetId::new(
                    alice_target.summary.target_id.as_str().to_string(),
                )
                .expect("target id"),
            ),
        },
        removed_run,
        removed_scope,
    )
    .await;
    assert_eq!(
        wait_for_triggered_delivery_outcome(delivery_store.as_ref(), removed_run).await,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Failed
    );
    assert!(
        harness.slack_messages().is_empty(),
        "removed extension must have no wire fallback"
    );

    // Reinstalling/reactivating the package is not personal connection
    // authority. Without a newly provisioned DM record, the pre-removal target
    // remains unavailable.
    dm_targets
        .delete(ADAPTER, &alice)
        .await
        .expect("clear personal target during removal");
    harness
        ._host
        .activate(ADAPTER)
        .await
        .expect("reactivate reinstalled channel extension");
    let (reinstalled_scope, reinstalled_run) = completed_trigger_run_for_user(
        &harness,
        &alice,
        "thread:reinstalled-unpaired-target",
        "must not resurrect stale target",
    )
    .await;
    hook.on_trigger_submitted(
        TriggerFire {
            identity: TriggerFireIdentity::new(
                TenantId::new(TENANT).expect("tenant"),
                TriggerId::new(),
                chrono::Utc::now(),
            ),
            creator_user_id: alice,
            agent_id: Some(AgentId::new(AGENT).expect("agent")),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            prompt: "reinstalled but unpaired".to_string(),
            delivery_target: Some(
                ironclaw_triggers::TriggerDeliveryTargetId::new(
                    alice_target.summary.target_id.as_str().to_string(),
                )
                .expect("target id"),
            ),
        },
        reinstalled_run,
        reinstalled_scope,
    )
    .await;
    assert_eq!(
        wait_for_triggered_delivery_outcome(delivery_store.as_ref(), reinstalled_run).await,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Failed
    );
    assert!(
        harness.slack_messages().is_empty(),
        "reinstall without renewed personal authority must not resurrect wire delivery"
    );
}

/// The shipping Telegram adapter participates in the generic target path:
/// its codec turns the pairing-provisioned DM record into the only binding
/// ref used by resolution and delivery. No test-only resolver or hand-built
/// Telegram ref is involved.
#[tokio::test]
async fn telegram_target_is_enumerated_resolved_and_delivered_through_generic_wiring() {
    let harness = build_harness_with_telegram(TurnMode::Running).await;
    let dm_targets = generic_dm_target_store();
    let user = UserId::new(USER).expect("user"); // safety: static test user id is valid.
    dm_targets
        .upsert(
            "telegram",
            &user,
            TELEGRAM_USER.to_string(),
            dm_target_payload(None, TELEGRAM_USER),
        )
        .await
        .expect("provision Telegram DM target"); // safety: in-memory store should not fail.
    let provider = Arc::new(generic_outbound_target_provider(&harness, dm_targets));

    let listed = provider
        .list_outbound_delivery_targets(&operator_caller())
        .await
        .expect("list targets");
    let telegram = listed
        .iter()
        .find(|entry| entry.summary.channel.as_str() == "telegram")
        .expect("Telegram personal target is enumerated");
    assert!(
        telegram
            .summary
            .target_id
            .as_str()
            .starts_with("telegram:personal-dm:")
    );
    assert_eq!(
        outbound_reply_target(telegram).as_str(),
        format!("tg:{TELEGRAM_USER}:_:_"),
        "the provider emits Telegram's canonical protocol binding"
    );

    let resolved = provider
        .resolve_outbound_delivery_target(&operator_caller(), &telegram.summary.target_id)
        .await
        .expect("resolve target")
        .expect("listed Telegram target resolves");
    assert_eq!(
        outbound_reply_target(&resolved),
        outbound_reply_target(telegram)
    );
    let resolved_by_binding = provider
        .resolve_reply_target_binding(&operator_caller(), &outbound_reply_target(&resolved))
        .await
        .expect("resolve generated binding")
        .expect("provider-generated Telegram binding resolves");
    assert_eq!(
        resolved_by_binding.summary.target_id,
        telegram.summary.target_id
    );

    let registry = Arc::new(crate::outbound::MutableOutboundDeliveryTargetRegistry::default());
    registry
        .register_provider(
            "channel",
            provider as Arc<dyn OutboundDeliveryTargetProvider>,
        )
        .expect("register target provider");
    registry
        .register_provider(
            "telegram-topicless-parent",
            Arc::new(
                ironclaw_outbound::HostOwnedOutboundDeliveryTargetProvider::new(
                    ironclaw_outbound::OutboundDeliveryTargetSummary::new(
                        ironclaw_outbound::OutboundDeliveryTargetId::new(
                            "telegram:shared-channel:-100123",
                        )
                        .expect("target id"),
                        "telegram",
                        "Telegram parent chat",
                        None,
                    )
                    .expect("target summary"),
                    ironclaw_outbound::DeliveryTargetCapabilities {
                        final_replies: true,
                        ..Default::default()
                    },
                    ironclaw_outbound::RunFinalReplyDestination::External {
                        reply_target_binding_ref: ReplyTargetBindingRef::new("tg:-100123:_:_")
                            .expect("topicless Telegram parent target"),
                    },
                ),
            ),
        )
        .expect("register topicless Telegram parent target");
    let current_targets = current_target_resolver(&harness.assembly, Arc::clone(&registry));

    let binding_service = harness
        .assembly
        .binding_service_for_extension_for_test("telegram")
        .expect("Telegram binding service");
    let adapter_id = ProductAdapterId::new("telegram").expect("adapter id");
    let installation_id =
        AdapterInstallationId::new(TELEGRAM_INSTALLATION).expect("installation id");
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".to_string(),
        },
        installation_id.as_str(),
    );
    let topic_context = TrustedInboundContext::from_verified_evidence(
        adapter_id.clone(),
        installation_id.clone(),
        chrono::Utc::now(),
        &evidence,
    )
    .expect("trusted Telegram topic context");
    let topic_parsed = ParsedProductInbound::new(
        ExternalEventId::new("telegram:event:implicit-topic-target").expect("event id"),
        ExternalActorRef::new("telegram_user", TELEGRAM_USER, None::<String>)
            .expect("Telegram actor"),
        ExternalConversationRef::new(None, "-100123", Some("77"), None)
            .expect("Telegram topic conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "send me a random number every minute",
                Vec::new(),
                ProductTriggerReason::BotMention,
            )
            .expect("Telegram topic message"),
        ),
    )
    .expect("parsed Telegram topic inbound");
    let topic_envelope = ProductInboundEnvelope::from_trusted_parse(topic_context, topic_parsed)
        .expect("Telegram topic envelope");
    let topic_binding = binding_service
        .resolve_binding(ResolveBindingRequest::from_envelope(&topic_envelope))
        .await
        .expect("Telegram topic binds");
    let topic_scope = ResourceScope {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        user_id: user.clone(),
        agent_id: Some(AgentId::new(AGENT).expect("agent")),
        project_id: Some(ProjectId::new(PROJECT).expect("project")),
        mission_id: None,
        thread_id: Some(topic_binding.thread_id),
        invocation_id: InvocationId::new(),
    };
    assert!(
        current_targets
            .resolve_current_target_id(&topic_scope, &topic_binding.reply_target_binding_ref)
            .await
            .expect("implicit topic target lookup")
            .is_none(),
        "a topic-scoped source must not downgrade to the topicless parent chat"
    );

    let context = TrustedInboundContext::from_verified_evidence(
        adapter_id,
        installation_id,
        chrono::Utc::now(),
        &evidence,
    )
    .expect("trusted Telegram context");
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new("telegram:event:implicit-trigger-target").expect("event id"),
        ExternalActorRef::new("telegram_user", TELEGRAM_USER, None::<String>)
            .expect("Telegram actor"),
        ExternalConversationRef::new(None, TELEGRAM_USER, None, None)
            .expect("Telegram conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "send me a random number every minute",
                Vec::new(),
                ProductTriggerReason::DirectChat,
            )
            .expect("Telegram message"),
        ),
    )
    .expect("parsed Telegram inbound");
    let envelope =
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("Telegram envelope");
    let source_binding = binding_service
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect("Telegram conversation binds");
    assert!(
        source_binding
            .reply_target_binding_ref
            .as_str()
            .starts_with("reply:"),
        "source runs carry the conversation store's opaque binding"
    );
    let source_scope = ResourceScope {
        tenant_id: TenantId::new(TENANT).expect("tenant"),
        user_id: user.clone(),
        agent_id: Some(AgentId::new(AGENT).expect("agent")),
        project_id: Some(ProjectId::new(PROJECT).expect("project")),
        mission_id: None,
        thread_id: Some(source_binding.thread_id),
        invocation_id: InvocationId::new(),
    };
    let inherited_target = current_targets
        .resolve_current_target_id(&source_scope, &source_binding.reply_target_binding_ref)
        .await
        .expect("implicit source target lookup")
        .expect("opaque Telegram source binding resolves to a current target");
    assert_eq!(
        inherited_target, telegram.summary.target_id,
        "implicit trigger delivery inherits the source Telegram DM"
    );

    let scope = foreign_run_scope();
    harness.ensure_scope_thread(&scope).await;
    let run_id = TurnRunId::new();
    harness
        .coordinator
        .complete_run(
            scope.clone(),
            TurnActor::new(user.clone()),
            run_id,
            "Telegram scheduled result",
            TurnOriginKind::ScheduledTrigger,
        )
        .await
        .expect("seed completed trigger run");

    let delivery_store = Arc::new(in_memory_backed_outbound_state_store());
    let hook = GenericTriggeredRunDeliveryHook::new(
        Arc::clone(&harness.assembly),
        Arc::clone(&delivery_store) as Arc<dyn TriggeredRunDeliveryStore>,
        harness.outbound.clone() as Arc<dyn CommunicationPreferenceRepository>,
        current_targets,
        Arc::clone(&harness.event_router),
    );
    let fire = TriggerFire {
        identity: TriggerFireIdentity::new(
            TenantId::new(TENANT).expect("tenant"), // safety: static test tenant id is valid.
            TriggerId::new(),
            chrono::Utc::now(),
        ),
        creator_user_id: user,
        agent_id: Some(AgentId::new(AGENT).expect("agent")), // safety: static test agent id is valid.
        project_id: None,
        prompt: "scheduled result to Telegram".to_string(),
        delivery_target: Some(
            ironclaw_triggers::TriggerDeliveryTargetId::new(
                telegram.summary.target_id.as_str().to_string(),
            )
            .expect("target id"),
        ),
    };
    use crate::automation::trigger_poller::PostSubmitDeliveryHook as _;
    hook.on_trigger_submitted(fire, run_id, scope).await;

    for _ in 0..200 {
        if delivery_store
            .load_triggered_run_delivery(run_id)
            .await
            .expect("load outcome")
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let record = delivery_store
        .load_triggered_run_delivery(run_id)
        .await
        .expect("load outcome")
        .expect("delivery outcome");
    assert_eq!(
        record.outcome,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Delivered
    );
    let messages = harness.telegram_messages();
    assert_eq!(messages.len(), 1, "one Telegram result: {messages:?}");
    assert_eq!(messages[0]["chat_id"], TELEGRAM_USER);
    assert!(
        messages[0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("Telegram scheduled result")),
        "final result reaches Telegram: {messages:?}"
    );
}
// arch-exempt: large_file, channel host end-to-end coverage remains centralized, plan #6175
