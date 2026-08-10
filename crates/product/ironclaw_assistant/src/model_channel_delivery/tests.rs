//! Unit tests for the model-called channel delivery port.
//!
//! Split out of `model_channel_delivery.rs` verbatim (crate precedent:
//! `channel_host/e2e_tests.rs`) so the production module stays reviewable;
//! behavior is unchanged.

use super::*;
use crate::{DeliveryRetryPolicy, NoReplyContext};
use ironclaw_extension_contracts::channel_adapter::{
    ChannelAdapter, ChannelError, DeliveryReport, InboundOutcome, OutboundEnvelope,
    PartDeliveryOutcome, VerifiedInbound,
};
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::preference_target::PreferenceTargetEncodeRequest;
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProjectId, TenantId, ThreadId, UserId},
    resource::ResourceScope,
};
use ironclaw_loop_contracts::RunProfileResolver;
use ironclaw_outbound::{
    DeliveryTargetCapabilities, OutboundDeliveryAttempt, OutboundDeliveryId,
    OutboundDeliveryStatus, OutboundDeliveryTargetEntry, OutboundDeliveryTargetId,
    OutboundDeliveryTargetOwner, OutboundDeliveryTargetRegistry, OutboundDeliveryTargetSummary,
    OutboundPushCandidate, OutboundPushKind,
};
use ironclaw_product_contracts::delivery::{ChannelDeliveryResolver, ResolvedChannelDelivery};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, ResumeTurnRequest,
    ResumeTurnResponse, RetryTurnRequest, RetryTurnResponse, RunProfileId, RunProfileVersion,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnAdmissionPolicy, TurnError,
    TurnId, TurnRunState, TurnStatus,
};
use std::collections::VecDeque as ReportQueue;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const TENANT: &str = "tenant-model-delivery";
const USER: &str = "user-model-delivery";
const AGENT: &str = "agent-model-delivery";
const PROJECT: &str = "project-model-delivery";
const THREAD: &str = "thread-model-delivery";

fn tenant() -> TenantId {
    TenantId::new(TENANT).expect("tenant")
}

fn user() -> UserId {
    UserId::new(USER).expect("user")
}

fn agent() -> AgentId {
    AgentId::new(AGENT).expect("agent")
}

fn project() -> ProjectId {
    ProjectId::new(PROJECT).expect("project")
}

fn thread() -> ThreadId {
    ThreadId::new(THREAD).expect("thread")
}

fn resource_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: tenant(),
        user_id: user(),
        agent_id: Some(agent()),
        project_id: Some(project()),
        mission_id: None,
        thread_id: Some(thread()),
        invocation_id: InvocationId::new(),
    }
}

fn base_request(
    target_id: OutboundDeliveryTargetId,
    content: impl Into<String>,
) -> ModelChannelDeliveryRequest {
    ModelChannelDeliveryRequest {
        scope: resource_scope(),
        run_id: RunId::new(),
        authenticated_actor_user_id: user(),
        target_id,
        content: content.into(),
    }
}

fn target_id(value: &str) -> OutboundDeliveryTargetId {
    OutboundDeliveryTargetId::new(value).expect("target id")
}

fn reply_ref(value: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(value).expect("reply target binding ref")
}

fn external_target_entry(id: &str, binding: &str) -> OutboundDeliveryTargetEntry {
    OutboundDeliveryTargetEntry {
        summary: OutboundDeliveryTargetSummary::new(
            target_id(id),
            "acme-chat",
            "Acme channel",
            None,
        )
        .expect("summary"),
        capabilities: DeliveryTargetCapabilities {
            final_replies: true,
            progress: false,
            gate_prompts: false,
            auth_prompts: false,
            notifications: true,
            modalities: Vec::new(),
        },
        destination: reply_ref(binding),
        owner: OutboundDeliveryTargetOwner::new(tenant(), user()),
    }
}

struct FakeTargetProvider(Vec<OutboundDeliveryTargetEntry>);

#[async_trait]
impl OutboundDeliveryTargetProvider for FakeTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        _scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        Ok(self.0.clone())
    }
}

fn registry(entries: Vec<OutboundDeliveryTargetEntry>) -> Arc<dyn OutboundDeliveryTargetProvider> {
    Arc::new(OutboundDeliveryTargetRegistry::new(vec![Arc::new(
        FakeTargetProvider(entries),
    )]))
}

/// A BARE provider — the trait's default `resolve_outbound_delivery_target`
/// filters on `final_replies` only, with no owner check. Injecting this
/// (rather than [`registry`]) is what proves owner scoping is the
/// service's own guarantee.
fn bare_provider(
    entries: Vec<OutboundDeliveryTargetEntry>,
) -> Arc<dyn OutboundDeliveryTargetProvider> {
    Arc::new(FakeTargetProvider(entries))
}

/// Scripted run-state lookup: either a fixed state, a `TurnError`, or a
/// panic on call (proving a scenario never needed the origin check).
enum ScriptedRunLookup {
    State(Box<TurnRunState>),
    Error,
    Unreachable,
}

struct ScriptedRunStateStore {
    lookup: ScriptedRunLookup,
}

#[async_trait]
impl AgentTurnRuntimePort for ScriptedRunStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        unreachable!("model channel delivery never submits turns")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        unreachable!("model channel delivery never resumes turns")
    }

    async fn retry_turn(&self, _request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        unreachable!("model channel delivery never retries turns")
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        unreachable!("model channel delivery never cancels runs")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        match &self.lookup {
            ScriptedRunLookup::State(state) => {
                let mut state = state.clone();
                state.scope = request.scope;
                state.run_id = request.run_id;
                Ok(*state)
            }
            ScriptedRunLookup::Error => Err(TurnError::Unavailable {
                reason: "scripted".to_string(),
            }),
            ScriptedRunLookup::Unreachable => {
                unreachable!("get_run_state should not be reached for this scenario")
            }
        }
    }
}

fn turn_run_state_with_reply_target(target: ReplyTargetBindingRef) -> Box<TurnRunState> {
    Box::new(TurnRunState {
        allow_steering: false,
        scope: TurnScope::new(tenant(), Some(agent()), Some(project()), thread()),
        actor: None,
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status: TurnStatus::Completed,
        accepted_message_ref: AcceptedMessageRef::new("msg:scripted").expect("ref"),
        source_binding_ref: SourceBindingRef::new("src:scripted").expect("ref"),
        reply_target_binding_ref: target,
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        model_usage: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(1),
        product_context: None,
        resume_disposition: None,
    })
}

struct RecordingChannelAdapter {
    envelopes: Mutex<Vec<OutboundEnvelope>>,
    reports: Mutex<ReportQueue<DeliveryReport>>,
}

impl RecordingChannelAdapter {
    fn new(reports: Vec<DeliveryReport>) -> Self {
        Self {
            envelopes: Mutex::new(Vec::new()),
            reports: Mutex::new(reports.into()),
        }
    }

    fn envelopes(&self) -> Vec<OutboundEnvelope> {
        self.envelopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl ChannelAdapter for RecordingChannelAdapter {
    fn inbound(&self, _request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        Ok(InboundOutcome::Ignore)
    }

    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        _egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        self.envelopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(envelope.clone());
        if let Some(report) = self
            .reports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
        {
            return Ok(report);
        }
        Ok(DeliveryReport {
            parts: envelope
                .parts
                .iter()
                .map(|_| PartDeliveryOutcome::Sent {
                    vendor_message_ref: Some("vendor-ref-1".to_string()),
                })
                .collect(),
        })
    }
}

struct DenyAllEgress;

#[async_trait]
impl RestrictedEgress for DenyAllEgress {
    async fn send(
        &self,
        _request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Err(RestrictedEgressError::PolicyDenied)
    }
}

/// Resolves any extension id to the shared recording adapter, except
/// `unknown-extension`, which yields no active channel (drives the
/// `ChannelUnavailable` coordinator error).
struct StaticResolver {
    adapter: Arc<RecordingChannelAdapter>,
}

impl ChannelDeliveryResolver for StaticResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        if extension_id == "unknown-extension" {
            return None;
        }
        Some(ResolvedChannelDelivery {
            extension_id: ironclaw_host_api::ids::ExtensionId::new(extension_id)
                .expect("extension id"),
            installation_id: ironclaw_host_api::product_adapter::AdapterInstallationId::new(
                "install-alpha",
            )
            .expect("installation id"),
            adapter: Arc::clone(&self.adapter) as Arc<dyn ChannelAdapter>,
            egress: Arc::new(DenyAllEgress),
        })
    }
}

struct StaticTargetResolver;

#[async_trait]
impl ProductOutboundTargetResolver for StaticTargetResolver {
    async fn resolve_product_outbound_target_metadata(
        &self,
        _target: &ValidatedReplyTargetBinding,
        _require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductSurfaceFailure> {
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: ExternalConversationRef::new(None, "conv-1", None, None)
                .expect("conversation ref"),
            external_actor_ref: None,
        })
    }
}

struct Harness {
    deliverer: CoordinatedModelChannelDelivery,
    adapter: Arc<RecordingChannelAdapter>,
    outbound_store: Arc<dyn OutboundStateStorePort>,
}

fn build_harness(
    entries: Vec<OutboundDeliveryTargetEntry>,
    lookup: ScriptedRunLookup,
    reports: Vec<DeliveryReport>,
) -> Harness {
    build_harness_with_retry(
        entries,
        lookup,
        reports,
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    )
}

fn build_harness_with_retry(
    entries: Vec<OutboundDeliveryTargetEntry>,
    lookup: ScriptedRunLookup,
    reports: Vec<DeliveryReport>,
    retry: DeliveryRetryPolicy,
) -> Harness {
    build_harness_with_parts(
        registry(entries),
        lookup,
        reports,
        retry,
        Arc::new(StaticTargetResolver),
    )
}

/// Harness over an explicitly injected target provider — used to prove the
/// service owner-scopes whatever it is handed.
fn build_harness_with_provider(
    provider: Arc<dyn OutboundDeliveryTargetProvider>,
    lookup: ScriptedRunLookup,
) -> Harness {
    build_harness_with_parts(
        provider,
        lookup,
        Vec::new(),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
        Arc::new(StaticTargetResolver),
    )
}

/// Harness over an explicitly injected `ProductOutboundTargetResolver` —
/// used to drive [`CodecChannelTargetResolver`] through its real caller
/// (`ValidatedReplyTargetBinding` is minted inside `ironclaw_outbound` and
/// cannot be constructed here, so the resolver is unreachable by a direct
/// unit call — see `.claude/rules/testing.md`, "Test through the caller").
fn build_harness_with_target_resolver(
    entries: Vec<OutboundDeliveryTargetEntry>,
    lookup: ScriptedRunLookup,
    target_resolver: Arc<dyn ProductOutboundTargetResolver>,
) -> Harness {
    build_harness_with_parts(
        registry(entries),
        lookup,
        Vec::new(),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
        target_resolver,
    )
}

fn build_harness_with_parts(
    registry: Arc<dyn OutboundDeliveryTargetProvider>,
    lookup: ScriptedRunLookup,
    reports: Vec<DeliveryReport>,
    retry: DeliveryRetryPolicy,
    target_resolver: Arc<dyn ProductOutboundTargetResolver>,
) -> Harness {
    let store = Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let outbound_store: Arc<dyn OutboundStateStorePort> = store.clone();
    let adapter = Arc::new(RecordingChannelAdapter::new(reports));
    let resolver = Arc::new(StaticResolver {
        adapter: Arc::clone(&adapter),
    });
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&outbound_store),
        resolver,
        Arc::new(NoReplyContext),
        retry,
    ));
    let run_state: Arc<dyn AgentTurnRuntimePort> = Arc::new(ScriptedRunStateStore { lookup });
    let deliverer = CoordinatedModelChannelDelivery::new(ModelChannelDeliveryDeps {
        project_filesystem: Arc::new(crate::NoProjectFilesystem),
        fallback_agent_id: ironclaw_host_api::ids::AgentId::new("model-delivery-test")
            .expect("agent id"),
        registry,
        coordinator,
        outbound_store: Arc::clone(&outbound_store),
        target_resolver,
        run_state,
    });
    Harness {
        deliverer,
        adapter,
        outbound_store,
    }
}

fn sample_attempt() -> OutboundDeliveryAttempt {
    OutboundDeliveryAttempt {
        delivery_id: OutboundDeliveryId::new(),
        scope: TurnScope::new(tenant(), Some(agent()), Some(project()), thread()),
        candidate: OutboundPushCandidate {
            tenant_id: tenant(),
            agent_id: Some(agent()),
            project_id: Some(project()),
            thread_id: thread(),
            turn_run_id: None,
            target: reply_ref("reply:acme-chat-1"),
            kind: OutboundPushKind::ModelDelivery,
            projection_ref: ProjectionUpdateRef::new("model-delivery:test").expect("ref"),
            requires_reply_target_revalidation: true,
        },
        status: OutboundDeliveryStatus::Delivered,
        attempted_at: Utc::now(),
        failure_kind: None,
    }
}

#[tokio::test]
async fn deliver_for_model_rejects_oversized_content() {
    let harness = build_harness(vec![], ScriptedRunLookup::Unreachable, vec![]);
    let oversized = "a".repeat(MODEL_DELIVERY_MAX_CONTENT_BYTES + 1);
    let request = base_request(target_id("acme-chat-1"), oversized);
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("oversized content must be rejected");
    assert_eq!(error, ModelChannelDeliveryError::ContentTooLarge);

    // Boundary: exactly MAX bytes must NOT trip ContentTooLarge (proves
    // `>` not `>=`); the empty registry still rejects it, but as
    // TargetUnavailable, proving the size gate passed.
    let exact = "a".repeat(MODEL_DELIVERY_MAX_CONTENT_BYTES);
    let request = base_request(target_id("acme-chat-1"), exact);
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("unknown target still rejected");
    assert_eq!(error, ModelChannelDeliveryError::TargetUnavailable);
}

#[tokio::test]
async fn deliver_for_model_rejects_unknown_target() {
    // Case 1: registry has no matching entry at all.
    let harness = build_harness(vec![], ScriptedRunLookup::Unreachable, vec![]);
    let request = base_request(target_id("does-not-exist"), "hello");
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("unknown target rejected");
    assert_eq!(error, ModelChannelDeliveryError::TargetUnavailable);

    // Case 2: the provider returns an entry owned by a DIFFERENT user.
    //
    // Injected as a BARE `OutboundDeliveryTargetProvider`, whose default
    // `resolve_outbound_delivery_target` filters on `final_replies` ONLY
    // and would hand the foreign entry straight back. Rejection here can
    // therefore only come from the owner-scoping registry view
    // `CoordinatedModelChannelDelivery::new` wraps around it — i.e. the
    // cross-user guarantee is the service's, not the caller's wiring.
    // Deleting that wrap turns this into a cross-user target read: the run
    // state below resolves and the adapter is live, so an unscoped lookup
    // DELIVERS to the other user's conversation instead of erroring.
    let mut foreign = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    foreign.owner =
        OutboundDeliveryTargetOwner::new(tenant(), UserId::new("someone-else").expect("user"));
    let harness = build_harness_with_provider(
        bare_provider(vec![foreign]),
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
    );
    let request = base_request(target_id("acme-chat-1"), "hello");
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("foreign-owned target rejected");
    assert_eq!(error, ModelChannelDeliveryError::TargetUnavailable);
    assert!(
        harness.adapter.envelopes().is_empty(),
        "a foreign-owned target must never reach a vendor adapter"
    );

    // Control for the same bare-provider path: an entry the caller DOES
    // own still resolves, so Case 2 is proving owner scoping rather than
    // "a bare provider never resolves anything".
    let owned = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    let harness = build_harness_with_provider(
        bare_provider(vec![owned]),
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
    );
    let request = base_request(target_id("acme-chat-1"), "hello");
    harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect("caller-owned target still resolves through a bare provider");
}

#[tokio::test]
async fn deliver_for_model_denies_origin_conversation_target() {
    // Same-origin conflict: the run's own reply-target binding equals the
    // requested explicit target.
    let entry = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    let harness = build_harness(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref(
            "reply:acme-chat-1",
        ))),
        vec![],
    );
    let request = base_request(target_id("acme-chat-1"), "hello");
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("origin target denied");
    assert_eq!(error, ModelChannelDeliveryError::OriginConversationTarget);

    // Run-state read error: fail closed as Unavailable.
    let entry = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    let harness = build_harness(vec![entry], ScriptedRunLookup::Error, vec![]);
    let request = base_request(target_id("acme-chat-1"), "hello");
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("run-state error fails closed");
    assert_eq!(error, ModelChannelDeliveryError::Unavailable);
}

#[tokio::test]
async fn deliver_for_model_enforces_per_run_cap() {
    let entry = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    let harness = build_harness(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        vec![],
    );
    let run_id = RunId::new();
    for attempt in 0..MODEL_DELIVERY_PER_RUN_CAP {
        let mut request = base_request(target_id("acme-chat-1"), "hello");
        request.run_id = run_id;
        harness
            .deliverer
            .deliver_for_model(request)
            .await
            .unwrap_or_else(|error| panic!("attempt {attempt} should succeed, got {error:?}"));
    }
    let mut request = base_request(target_id("acme-chat-1"), "hello");
    request.run_id = run_id;
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("cap must be enforced");
    assert_eq!(error, ModelChannelDeliveryError::DeliveryCapExceeded);
}

#[tokio::test]
async fn deliver_for_model_scope_without_thread_id_fails_closed() {
    let entry = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    let harness = build_harness(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        vec![],
    );
    let mut request = base_request(target_id("acme-chat-1"), "hello");
    request.scope.thread_id = None;
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("an invocation scope with no thread id must fail closed");
    assert_eq!(error, ModelChannelDeliveryError::Unavailable);
    assert!(
        harness.adapter.envelopes().is_empty(),
        "deny-before-lookup: nothing may reach the adapter"
    );
}

/// FIFO eviction of the per-run cap ledger: after `MODEL_DELIVERY_TRACKED_RUNS_CAP`
/// distinct newer runs, the oldest run's counter is evicted and reset — a
/// re-delivery for it starts from zero instead of tripping the stale count.
#[tokio::test]
async fn deliver_for_model_fifo_eviction_resets_an_evicted_runs_counter() {
    let entry = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    let harness = build_harness(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        vec![],
    );
    let evicted_run = RunId::new();
    // Exhaust the evicted run's cap so a retained counter would reject it.
    for _ in 0..MODEL_DELIVERY_PER_RUN_CAP {
        let mut request = base_request(target_id("acme-chat-1"), "hello");
        request.run_id = evicted_run;
        harness
            .deliverer
            .deliver_for_model(request)
            .await
            .expect("filling the evicted run's cap succeeds");
    }
    // Push the ledger past its tracked-runs cap with distinct newer runs.
    for _ in 0..MODEL_DELIVERY_TRACKED_RUNS_CAP {
        let mut request = base_request(target_id("acme-chat-1"), "hello");
        request.run_id = RunId::new();
        harness
            .deliverer
            .deliver_for_model(request)
            .await
            .expect("distinct newer runs deliver");
    }
    // The evicted run's counter must have been reset with its eviction.
    let mut request = base_request(target_id("acme-chat-1"), "hello");
    request.run_id = evicted_run;
    harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect("an evicted run's counter restarts from zero");
}

#[tokio::test]
async fn deliver_for_model_returns_provider_evidence() {
    let entry = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    let harness = build_harness(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        vec![],
    );
    let request = base_request(target_id("acme-chat-1"), "hello there");
    let run_id = request.run_id;
    let invocation_id = request.scope.invocation_id;
    let evidence = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect("happy path delivers");
    assert_eq!(evidence.target.target_id.as_str(), "acme-chat-1");
    assert_eq!(
        evidence.provider_message_refs,
        vec!["vendor-ref-1".to_string()]
    );

    let envelopes = harness.adapter.envelopes();
    assert_eq!(envelopes.len(), 1);
    assert_eq!(envelopes[0].extension_id, "acme-chat");
    match &envelopes[0].parts[0] {
        OutboundPart::Text(text) => assert_eq!(text, "hello there"),
        other => panic!("expected text part, got {other:?}"),
    }

    // Contract 5's distinguishing facts, pinned directly against the
    // persisted attempt: a ModelDelivery push must stay
    // audit-distinguishable from an ordinary FinalReply push, not just
    // land on the same policy-class target. Without this, swapping
    // RunNotificationEventKind::ModelDelivery for FinalReplyReady (and
    // DeliveryIntent::ModelDelivery for FinalReply, and the
    // projection-ref format for anything else) would leave every other
    // assertion in this test green.
    let scope = TurnScope::new_with_owner(
        tenant(),
        Some(agent()),
        Some(project()),
        thread(),
        Some(user()),
    );
    let attempts = harness
        .outbound_store
        .list_delivery_attempts(scope)
        .await
        .expect("list delivery attempts");
    assert_eq!(
        attempts.len(),
        1,
        "expected exactly one persisted delivery attempt"
    );
    let expected_turn_run_id = TurnRunId::from_uuid(run_id.as_uuid());
    let attempt = &attempts[0];
    assert_eq!(attempt.candidate.kind, OutboundPushKind::ModelDelivery);
    assert_eq!(
        attempt.candidate.projection_ref.as_str(),
        format!("model-delivery:{run_id}:{invocation_id}")
    );
    assert_eq!(attempt.candidate.turn_run_id, Some(expected_turn_run_id));
}

#[tokio::test]
async fn deliver_for_model_maps_terminal_failure_kinds() {
    // Pure-function coverage for every branch, including the two
    // (Rejected / NoDelivery) that are structurally unreachable through
    // this port's own correct wiring — see the module doc comment on
    // `classify_delivery_outcome`.
    let target = OutboundDeliveryTargetSummary::new(
        target_id("acme-chat-1"),
        "acme-chat",
        "Acme channel",
        None,
    )
    .expect("summary");

    let delivered = classify_delivery_outcome(
        target.clone(),
        CoordinatedDeliveryOutcome::Delivered {
            attempt: sample_attempt(),
            conversation: ExternalConversationRef::new(None, "conv-1", None, None).expect("conv"),
            vendor_message_refs: vec!["ref-1".to_string()],
        },
    );
    assert_eq!(
        delivered,
        Ok(ModelChannelDeliveryEvidence {
            target: target.clone(),
            provider_message_refs: vec!["ref-1".to_string()],
            durably_recorded: true,
            already_delivered: false,
        })
    );

    let delivered_unconfirmed = classify_delivery_outcome(
        target.clone(),
        CoordinatedDeliveryOutcome::DeliveredUnconfirmed {
            attempt: sample_attempt(),
            conversation: ExternalConversationRef::new(None, "conv-1", None, None).expect("conv"),
            vendor_message_refs: vec!["ref-unconfirmed".to_string()],
        },
    );
    assert_eq!(
        delivered_unconfirmed,
        Ok(ModelChannelDeliveryEvidence {
            target: target.clone(),
            provider_message_refs: vec!["ref-unconfirmed".to_string()],
            durably_recorded: false,
            already_delivered: false,
        }),
        "provider evidence must survive while the failed terminal write stays explicit"
    );

    // A replay of a durably confirmed delivery. The ledger row does not retain
    // provider refs, so the empty list is honest — but it must be flagged, or
    // the caller reads "no refs" as unverified and resends what was already
    // sent, defeating the at-most-once claim.
    let already_delivered = classify_delivery_outcome(
        target.clone(),
        CoordinatedDeliveryOutcome::AlreadyDelivered {
            attempt: sample_attempt(),
        },
    );
    assert_eq!(
        already_delivered,
        Ok(ModelChannelDeliveryEvidence {
            target: target.clone(),
            provider_message_refs: Vec::new(),
            durably_recorded: true,
            already_delivered: true,
        }),
        "a durable replay must be distinguishable from a send with no provider reference"
    );

    let rejected = classify_delivery_outcome(
        target.clone(),
        CoordinatedDeliveryOutcome::Rejected {
            attempt: sample_attempt(),
        },
    );
    assert_eq!(rejected, Err(ModelChannelDeliveryError::Rejected));

    for kind in [
        DeliveryFailureKind::AuthorizationRevoked,
        DeliveryFailureKind::TransientValidatorError,
        DeliveryFailureKind::TransportUnavailable,
        DeliveryFailureKind::RateLimited,
        DeliveryFailureKind::Rejected,
        DeliveryFailureKind::Unknown,
    ] {
        let failed = classify_delivery_outcome(
            target.clone(),
            CoordinatedDeliveryOutcome::Failed {
                attempt: sample_attempt(),
                failure_kind: kind,
            },
        );
        assert_eq!(failed, Err(ModelChannelDeliveryError::Failed { kind }));
    }

    let no_delivery =
        classify_delivery_outcome(target.clone(), CoordinatedDeliveryOutcome::NoDelivery);
    assert_eq!(no_delivery, Err(ModelChannelDeliveryError::Internal));

    let channel_unavailable =
        classify_coordinator_error(CoordinatedDeliveryError::ChannelUnavailable {
            extension_id: "acme-chat".to_string(),
        });
    assert_eq!(
        channel_unavailable,
        ModelChannelDeliveryError::Failed {
            kind: DeliveryFailureKind::TransportUnavailable
        }
    );

    // A concurrent duplicate is model-correctable (retry later / don't
    // duplicate), so it must stay model-visible rather than masking as an
    // internal host fault (rules/tools.md).
    let other = classify_coordinator_error(CoordinatedDeliveryError::AlreadyInFlight);
    assert_eq!(
        other,
        ModelChannelDeliveryError::Failed {
            kind: DeliveryFailureKind::Rejected
        }
    );
    let dm_guard = classify_coordinator_error(CoordinatedDeliveryError::Workflow(
        crate::ProductSurfaceFailure::OutboundTargetNotDirectMessage,
    ));
    assert_eq!(dm_guard, ModelChannelDeliveryError::Rejected);

    // End-to-end proof for a reachable failure-kind mapping: an adapter
    // that reports a permanent per-part failure drives the real
    // coordinator to `Failed { kind: Rejected }` (OUT-7: partial/whole
    // permanent failure is terminal, never retried).
    let entry = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    let harness = build_harness(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        vec![DeliveryReport {
            parts: vec![PartDeliveryOutcome::Permanent {
                reason: "vendor rejected the message".to_string(),
            }],
        }],
    );
    let request = base_request(target_id("acme-chat-1"), "hello");
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("permanent adapter failure maps to Failed");
    assert_eq!(
        error,
        ModelChannelDeliveryError::Failed {
            kind: DeliveryFailureKind::Rejected
        }
    );

    // And the `ChannelUnavailable` coordinator error, end to end: the
    // registry entry's channel resolves to nothing active.
    let mut unresolvable = external_target_entry("acme-chat-1", "reply:acme-chat-1");
    unresolvable.summary = OutboundDeliveryTargetSummary::new(
        target_id("acme-chat-1"),
        "unknown-extension",
        "Acme channel",
        None,
    )
    .expect("summary");
    let harness = build_harness(
        vec![unresolvable],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        vec![],
    );
    let request = base_request(target_id("acme-chat-1"), "hello");
    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("unresolvable channel maps to Failed");
    assert_eq!(
        error,
        ModelChannelDeliveryError::Failed {
            kind: DeliveryFailureKind::TransportUnavailable
        }
    );
}

/// Records how many calls it saw and reports its own label back through
/// `provider_message_refs`, so a test can tell WHICH binding served a call.
struct LabelledModelChannelDelivery {
    label: &'static str,
    calls: AtomicUsize,
}

impl LabelledModelChannelDelivery {
    fn new(label: &'static str) -> Self {
        Self {
            label,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ModelChannelDelivery for LabelledModelChannelDelivery {
    async fn deliver_for_model(
        &self,
        _request: ModelChannelDeliveryRequest,
    ) -> Result<ModelChannelDeliveryEvidence, ModelChannelDeliveryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ModelChannelDeliveryEvidence {
            target: OutboundDeliveryTargetSummary::new(
                target_id("acme-chat-1"),
                "acme-chat",
                "Acme channel",
                None,
            )
            .expect("summary"),
            provider_message_refs: vec![self.label.to_string()],
            durably_recorded: true,
            already_delivered: false,
        })
    }
}

#[tokio::test]
async fn deferred_model_channel_delivery_fails_closed_until_bound() {
    // The registry-assembly ordering means the slot is REGISTERED before
    // composition can build the coordinator, and a composition path with
    // no channel egress transport never binds it at all. Until it is
    // bound, the tool must be as unavailable as the host-runtime default
    // it replaces — never a panic and never a silent success.
    let deferred = DeferredModelChannelDelivery::new();
    let error = deferred
        .deliver_for_model(base_request(target_id("acme-chat-1"), "hello"))
        .await
        .expect_err("an unbound slot must fail closed");
    assert_eq!(error, ModelChannelDeliveryError::Unavailable);

    // Binding flips it live through the SAME slot handle a registered
    // handler holds.
    let bound = Arc::new(LabelledModelChannelDelivery::new("bound"));
    assert!(deferred.bind(Arc::clone(&bound) as Arc<dyn ModelChannelDelivery>));
    let evidence = deferred
        .deliver_for_model(base_request(target_id("acme-chat-1"), "hello"))
        .await
        .expect("a bound slot delegates");
    assert_eq!(evidence.provider_message_refs, vec!["bound".to_string()]);
    assert_eq!(bound.calls(), 1);
}

#[tokio::test]
async fn deferred_model_channel_delivery_keeps_the_first_binding() {
    // First write wins (the `channel_disconnect_slot` contract this
    // mirrors). A second bind must not silently repoint an already-live
    // capability at a different delivery service mid-composition.
    let first = Arc::new(LabelledModelChannelDelivery::new("first"));
    let second = Arc::new(LabelledModelChannelDelivery::new("second"));
    let deferred = DeferredModelChannelDelivery::new();

    assert!(deferred.bind(Arc::clone(&first) as Arc<dyn ModelChannelDelivery>));
    assert!(
        !deferred.bind(Arc::clone(&second) as Arc<dyn ModelChannelDelivery>),
        "a second bind must report that it was ignored"
    );

    let evidence = deferred
        .deliver_for_model(base_request(target_id("acme-chat-1"), "hello"))
        .await
        .expect("calls still route through the slot");
    assert_eq!(evidence.provider_message_refs, vec!["first".to_string()]);
    assert_eq!(first.calls(), 1);
    assert_eq!(
        second.calls(),
        0,
        "the discarded binding must never serve a call"
    );
}

/// Decodes only its own `reply:fake-codec:<conversation>` grammar.
struct FakeGrammarCodec;

impl PreferenceTargetCodec for FakeGrammarCodec {
    fn conversation_for_target(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<ExternalConversationRef> {
        let conversation = target.as_str().strip_prefix("reply:fake-codec:")?;
        ExternalConversationRef::new(None::<&str>, conversation, None, None).ok()
    }

    fn is_personal_direct_message(&self, _target: &ReplyTargetBindingRef) -> bool {
        false
    }

    fn direct_message_actor_for_target(&self, _target: &ReplyTargetBindingRef) -> Option<String> {
        None
    }

    fn encode_shared_conversation_target(
        &self,
        _request: PreferenceTargetEncodeRequest<'_>,
    ) -> Option<ReplyTargetBindingRef> {
        None
    }

    fn encode_personal_direct_message_target(
        &self,
        _request: PreferenceTargetEncodeRequest<'_>,
        _external_actor_id: &str,
    ) -> Option<ReplyTargetBindingRef> {
        None
    }
}

#[tokio::test]
async fn codec_channel_target_resolver_decodes_or_fails_closed() {
    // Driven through the real caller: `ValidatedReplyTargetBinding` is
    // minted inside `ironclaw_outbound` from a validated claim and cannot
    // be constructed from this crate, so the resolver is only reachable
    // via `DeliveryCoordinator::deliver`.

    // Decodable ref: the resolver — not the target entry, which carries no
    // conversation — is what supplies the destination conversation.
    let mut entry = external_target_entry("acme-chat-1", "reply:fake-codec:conv-77");
    entry.summary =
        OutboundDeliveryTargetSummary::new(target_id("acme-chat-1"), "acme-chat", "Acme", None)
            .expect("summary");
    let harness = build_harness_with_target_resolver(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        Arc::new(CodecChannelTargetResolver::new(vec![Arc::new(
            FakeGrammarCodec,
        )])),
    );
    harness
        .deliverer
        .deliver_for_model(base_request(target_id("acme-chat-1"), "hello"))
        .await
        .expect("a decodable binding delivers");
    let envelopes = harness.adapter.envelopes();
    assert_eq!(envelopes.len(), 1);
    assert_eq!(
        envelopes[0].target.conversation.conversation_id(),
        "conv-77",
        "the codec-decoded conversation must be the delivery destination"
    );

    // Undecodable ref (no registered codec owns this grammar): the
    // resolver must fail closed rather than fall through to some other
    // channel's conversation. Observable at this port as `Internal`, and —
    // the fact that matters — NOTHING reached the vendor adapter, while
    // the attempt is still recorded terminally for audit.
    let entry = external_target_entry("acme-chat-1", "reply:unknown-grammar:conv-77");
    let harness = build_harness_with_target_resolver(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        Arc::new(CodecChannelTargetResolver::new(vec![Arc::new(
            FakeGrammarCodec,
        )])),
    );
    let error = harness
        .deliverer
        .deliver_for_model(base_request(target_id("acme-chat-1"), "hello"))
        .await
        .expect_err("an undecodable binding must fail closed");
    assert_eq!(error, ModelChannelDeliveryError::Internal);
    assert!(
        harness.adapter.envelopes().is_empty(),
        "an undecodable binding must never reach a vendor adapter"
    );
    let attempts = harness
        .outbound_store
        .list_delivery_attempts(TurnScope::new_with_owner(
            tenant(),
            Some(agent()),
            Some(project()),
            thread(),
            Some(user()),
        ))
        .await
        .expect("list delivery attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, OutboundDeliveryStatus::Failed);

    // An empty codec set is the same fail-closed answer, not a panic.
    let entry = external_target_entry("acme-chat-1", "reply:fake-codec:conv-77");
    let harness = build_harness_with_target_resolver(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        Arc::new(CodecChannelTargetResolver::new(Vec::new())),
    );
    let error = harness
        .deliverer
        .deliver_for_model(base_request(target_id("acme-chat-1"), "hello"))
        .await
        .expect_err("no codecs at all must fail closed");
    assert_eq!(error, ModelChannelDeliveryError::Internal);
    assert!(harness.adapter.envelopes().is_empty());
}

/// LEGACY kernel shape: threads created before run-acts-as-invoker could be
/// owned by one user (the retired shared-route "subject") while a different
/// authenticated actor drives the run, so owner ≠ actor rows still exist. The
/// catalog this tool may reach must follow the ACTOR, never the stored owner.
///
/// Otherwise any participant acting on such a legacy thread could name the
/// legacy owner's target ids and push bot-identity content into that owner's
/// own destinations — their personal DM included — from a conversation the
/// owner may never read.
#[tokio::test]
async fn a_shared_route_participant_cannot_reach_a_legacy_owners_targets() {
    let subject = UserId::new("route-subject").expect("subject id");
    let participant = UserId::new("route-participant").expect("participant id");

    // The catalog entry belongs to the ROUTE SUBJECT.
    let mut entry = external_target_entry("acme-chat-1", "reply:fake-codec:subject-dm");
    entry.owner = OutboundDeliveryTargetOwner::new(tenant(), subject.clone());

    let harness = build_harness_with_target_resolver(
        vec![entry],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        Arc::new(CodecChannelTargetResolver::new(vec![Arc::new(
            FakeGrammarCodec,
        )])),
    );

    // The run is scoped to the subject (shared-route owner) but driven by a
    // different authenticated actor.
    let mut request = base_request(target_id("acme-chat-1"), "leak attempt");
    request.scope.user_id = subject.clone();
    request.authenticated_actor_user_id = participant.clone();

    let error = harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect_err("a participant must not resolve the route subject's targets");
    assert_eq!(
        error,
        ModelChannelDeliveryError::TargetUnavailable,
        "the subject's target must be indistinguishable from one that does not exist"
    );
    assert!(
        harness.adapter.envelopes().is_empty(),
        "nothing may reach a vendor adapter for another user's target"
    );

    // Control: the same call by the target's own owner still delivers, so the
    // guard is scoping the catalog rather than breaking delivery outright.
    let mut owned = external_target_entry("acme-chat-1", "reply:fake-codec:subject-dm");
    owned.owner = OutboundDeliveryTargetOwner::new(tenant(), subject.clone());
    let harness = build_harness_with_target_resolver(
        vec![owned],
        ScriptedRunLookup::State(turn_run_state_with_reply_target(reply_ref("reply:origin"))),
        Arc::new(CodecChannelTargetResolver::new(vec![Arc::new(
            FakeGrammarCodec,
        )])),
    );
    let mut request = base_request(target_id("acme-chat-1"), "own delivery");
    request.scope.user_id = subject.clone();
    request.authenticated_actor_user_id = subject;
    harness
        .deliverer
        .deliver_for_model(request)
        .await
        .expect("the target's own owner still delivers");
    assert_eq!(harness.adapter.envelopes().len(), 1);
}
