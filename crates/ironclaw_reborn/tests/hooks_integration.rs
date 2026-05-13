//! End-to-end integration tests proving that `RebornLoopDriverHostFactory`
//! wires the `HookDispatcher` into the capability port seam correctly.
//!
//! These tests drive `host.invoke_capability(...)` against a host built via
//! `RebornLoopDriverHostFactory::build_text_only_host_with_capabilities`.
//! That exercises the same wrapping composition production code uses, so a
//! regression in the factory's hook wiring will surface here, whereas a unit
//! test against `HookedLoopCapabilityPort` alone (already present in
//! `ironclaw_hooks`) would not.
//!
//! Coverage:
//!
//! 1. With a `HookDispatcher` installed and a predicate-backed deny hook
//!    targeting `cap.blocked`, invoking `cap.blocked` is short-circuited at
//!    the hook seam and never reaches the inner port.
//! 2. With a `HookDispatcher` installed that contains a privileged selective
//!    hook (deny only when `cap.blocked`), invoking `cap.allowed` passes
//!    through to the inner port and completes normally — proving the
//!    middleware does not blanket-deny.
//! 3. With NO `HookDispatcher` (default factory shape), `cap.blocked` reaches
//!    the inner port — proving the hook plumbing is opt-in.
//!
//! Deferred coverage: predicate-pass "no opinion" currently denies with
//! `hook_predicate_pass` (see `installed_hook.rs` TODO). Once the dispatcher
//! grows an explicit `pass()` for restricted sinks, an additional test using
//! a `PredicateBackedBeforeCapabilityHook` against `cap.allowed` should be
//! added to prove non-matching predicate invocations also reach the inner
//! port.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_hooks::dispatch::HookDispatcher;
use ironclaw_hooks::evaluator::PredicateEvaluator;
use ironclaw_hooks::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
use ironclaw_hooks::installed_hook::PredicateBackedBeforeCapabilityHook;
use ironclaw_hooks::ordering::HookPhase;
use ironclaw_hooks::points::BeforeCapabilityHookContext;
use ironclaw_hooks::predicate::{CapabilityPredicate, HookPredicateSpec};
use ironclaw_hooks::registry::HookRegistry;
use ironclaw_hooks::sink::{
    PrivilegedBeforeCapabilityHook, PrivilegedGateSink, RestrictedBeforeCapabilityHook,
    RestrictedGateSink,
};
use ironclaw_host_api::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_reborn::{
    RebornLoopDriverHostFactory, RebornLoopDriverHostRequest, TextOnlyLoopHostConfig,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_turns::LoopResultRef;
use ironclaw_turns::{
    AcceptedMessageRef, EventCursor, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryRunProfileResolver, ReplyTargetBindingRef, RunProfileId, RunProfileResolutionRequest,
    RunProfileResolver, RunProfileVersion, SourceBindingRef, TurnLeaseToken, TurnRunId,
    TurnRunnerId, TurnScope, TurnStatus,
    run_profile::{
        AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome,
        CapabilityDeniedReasonKind, CapabilityDescriptorView, CapabilityInputRef,
        CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage, CapabilitySurfaceVersion,
        InMemoryLoopHostMilestoneSink, LoopCapabilityPort, LoopHostMilestoneKind, LoopRunContext,
        RunScopedHookMilestoneSink, VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
    runner::ClaimedTurnRun,
};

// ─── Inner-port stub ───────────────────────────────────────────────────────

/// Inner capability port stub that records every invocation and reports a
/// single `cap.allowed` / `cap.blocked` capability on the surface. Invocation
/// always completes successfully so we can prove that *not* reaching the
/// inner port is meaningful (i.e., the hook intercepted).
struct RecordingCapabilityPort {
    invocations: Mutex<Vec<CapabilityId>>,
    surface_version: CapabilitySurfaceVersion,
}

impl RecordingCapabilityPort {
    fn new() -> Self {
        Self {
            invocations: Mutex::new(Vec::new()),
            surface_version: CapabilitySurfaceVersion::new("hooks-integration:v1")
                .expect("surface version literal is valid"),
        }
    }

    fn invocations(&self) -> Vec<CapabilityId> {
        self.invocations
            .lock()
            .expect("invocations mutex not poisoned")
            .clone()
    }
}

#[async_trait]
impl LoopCapabilityPort for RecordingCapabilityPort {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        // Surface contains both capabilities used in the tests so the
        // factory's startup-time `visible_capabilities()` probe sees a valid
        // (non-empty) surface and registers the version.
        Ok(VisibleCapabilitySurface {
            version: self.surface_version.clone(),
            descriptors: vec![descriptor("cap.blocked"), descriptor("cap.allowed")],
        })
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.invocations
            .lock()
            .expect("invocations mutex not poisoned")
            .push(request.capability_id.clone());
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: LoopResultRef::new(format!("result:{}", request.capability_id))
                .expect("result ref literal is valid"),
            safe_summary: "stub capability completed".to_string(),
        }))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::with_capacity(request.invocations.len());
        for invocation in request.invocations {
            outcomes.push(self.invoke_capability(invocation).await?);
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension: false,
        })
    }
}

fn descriptor(capability_id: &str) -> CapabilityDescriptorView {
    CapabilityDescriptorView {
        capability_id: CapabilityId::new(capability_id).expect("capability id literal is valid"),
        provider: None,
        runtime: ironclaw_host_api::RuntimeKind::Wasm,
        safe_name: capability_id.to_string(),
        safe_description: format!("test capability {capability_id}"),
    }
}

// ─── Model-gateway stub ────────────────────────────────────────────────────

/// Minimal `HostManagedModelGateway` stub. The integration tests don't drive
/// the model port; the gateway is only required because the factory's type
/// signature demands one. Its `stream_model` is therefore never invoked.
struct UnusedGateway;

#[async_trait]
impl HostManagedModelGateway for UnusedGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // If this ever runs, the test is exercising the wrong seam.
        panic!("model gateway must not be invoked by capability-port integration tests");
    }
}

// ─── Hook implementations used by the tests ────────────────────────────────

/// Privileged builtin hook that denies only when the capability name matches
/// the configured target. Used to prove that non-matching invocations reach
/// the inner port through the wrapping seam.
struct SelectiveDenyHook {
    target: String,
}

#[async_trait]
impl PrivilegedBeforeCapabilityHook for SelectiveDenyHook {
    async fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn PrivilegedGateSink) {
        if ctx.capability_name == self.target {
            sink.deny("selective_deny_target_matched");
        } else {
            sink.allow();
        }
    }
}

/// Installed-tier hook that always pause-approves. Used to prove the
/// hook-middleware seam surfaces `PauseApproval` as
/// `CapabilityOutcome::ApprovalRequired` with a real `LoopGateRef`, rather
/// than the previous degraded `Denied` mapping.
struct PauseApprovalHook;

#[async_trait]
impl RestrictedBeforeCapabilityHook for PauseApprovalHook {
    async fn evaluate(
        &self,
        _ctx: &BeforeCapabilityHookContext,
        sink: &mut dyn RestrictedGateSink,
    ) {
        sink.pause_approval("integration-test pause approval");
    }
}

fn pause_approval_dispatcher() -> Arc<HookDispatcher> {
    let hook_id = HookId::derive(
        &ExtensionId("integration-tests".to_string()),
        "0.0.1",
        &HookLocalId("pause-approval".to_string()),
        HookVersion::ONE,
    );
    let mut dispatcher = HookDispatcher::new(HookRegistry::new());
    dispatcher
        .install_installed_before_capability(
            hook_id,
            HookPhase::Policy,
            Box::new(PauseApprovalHook),
        )
        .expect("install pause-approval hook");
    Arc::new(dispatcher)
}

fn predicate_deny_dispatcher() -> Arc<HookDispatcher> {
    // PredicateBackedBeforeCapabilityHook is the Installed-tier predicate
    // wrapper. Use the public Installed-tier installer, which constructs the
    // binding with HookTrustClass::Installed and routes the impl into the
    // Restricted variant — there is no public path that pairs Installed with
    // a Privileged impl.
    let hook_id = HookId::derive(
        &ExtensionId("integration-tests".to_string()),
        "0.0.1",
        &HookLocalId("deny-cap-blocked".to_string()),
        HookVersion::ONE,
    );
    let spec = HookPredicateSpec::DenyCapability {
        when: CapabilityPredicate::NameEquals {
            name: "cap.blocked".to_string(),
        },
        reason: "integration-test deny rule".to_string(),
    };
    let evaluator = Arc::new(PredicateEvaluator::new());
    let hook = PredicateBackedBeforeCapabilityHook::new(hook_id, spec, evaluator);

    let mut dispatcher = HookDispatcher::new(HookRegistry::new());
    dispatcher
        .install_installed_before_capability(hook_id, HookPhase::Policy, Box::new(hook))
        .expect("Installed-tier predicate hook installs at policy phase");
    Arc::new(dispatcher)
}

fn selective_deny_dispatcher(target: &str) -> Arc<HookDispatcher> {
    // SelectiveDenyHook is a Privileged (Builtin-tier) hook so it may mint
    // .allow() — which is exactly what we need to prove pass-through.
    let hook_id = HookId::for_builtin("tests::hooks_integration::selective_deny", HookVersion::ONE);
    let hook = SelectiveDenyHook {
        target: target.to_string(),
    };
    let mut dispatcher = HookDispatcher::new(HookRegistry::new());
    dispatcher
        .install_builtin_before_capability(hook_id, HookPhase::Policy, Box::new(hook))
        .expect("Builtin-tier hook installs at policy phase");
    Arc::new(dispatcher)
}

// ─── Fixture for building hosts with the factory ───────────────────────────

struct Fixture {
    thread_service: Arc<InMemorySessionThreadService>,
    checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    loop_checkpoint_store: Arc<InMemoryLoopCheckpointStore>,
    milestone_sink: Arc<InMemoryLoopHostMilestoneSink>,
    gateway: Arc<UnusedGateway>,
    thread_scope: ThreadScope,
    claimed: ClaimedTurnRun,
    context: LoopRunContext,
    surface_version: CapabilitySurfaceVersion,
}

impl Fixture {
    async fn new() -> Self {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
        let gateway = Arc::new(UnusedGateway);

        let tenant_id =
            TenantId::new("tenant-hooks-integration").expect("tenant id literal is valid");
        let agent_id = AgentId::new("agent-hooks-integration").expect("agent id literal is valid");
        let project_id =
            ProjectId::new("project-hooks-integration").expect("project id literal is valid");
        let user_id = UserId::new("user-hooks-integration").expect("user id literal is valid");
        let thread_id =
            ThreadId::new("thread-hooks-integration").expect("thread id literal is valid");
        let thread_scope = ThreadScope {
            tenant_id: tenant_id.clone(),
            agent_id: agent_id.clone(),
            project_id: Some(project_id.clone()),
            owner_user_id: None,
            mission_id: None,
        };
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: user_id.to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure_thread succeeds");
        thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                actor_id: user_id.to_string(),
                source_binding_id: Some("source-test".to_string()),
                reply_target_binding_id: Some("reply-test".to_string()),
                external_event_id: Some("event-hooks-integration".to_string()),
                content: MessageContent::text("hello hooks"),
            })
            .await
            .expect("accept_inbound_message succeeds");

        let turn_scope = TurnScope::new(
            tenant_id,
            Some(agent_id),
            Some(project_id),
            thread_id.clone(),
        );
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("interactive default run profile resolves");
        let turn_id = ironclaw_turns::TurnId::new();
        let run_id = TurnRunId::new();
        let state = ironclaw_turns::TurnRunState {
            scope: turn_scope.clone(),
            turn_id,
            run_id,
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new("accepted-hooks-integration")
                .expect("accepted message ref literal is valid"),
            source_binding_ref: SourceBindingRef::new("source-test")
                .expect("source binding ref literal is valid"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-test")
                .expect("reply target binding ref literal is valid"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: EventCursor(1),
        };
        let claimed = ClaimedTurnRun {
            state,
            resolved_run_profile: resolved.clone(),
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
        };
        let context = LoopRunContext::new(turn_scope, turn_id, run_id, resolved);

        Self {
            thread_service,
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink,
            gateway,
            thread_scope,
            claimed,
            context,
            surface_version: CapabilitySurfaceVersion::new("hooks-integration:v1")
                .expect("surface version literal is valid"),
        }
    }

    fn factory(&self) -> RebornLoopDriverHostFactory<InMemorySessionThreadService, UnusedGateway> {
        RebornLoopDriverHostFactory::new(
            Arc::clone(&self.thread_service),
            self.thread_scope.clone(),
            Arc::clone(&self.gateway),
            Arc::clone(&self.checkpoint_state_store) as _,
            Arc::clone(&self.loop_checkpoint_store) as _,
            Arc::clone(&self.milestone_sink) as _,
            TextOnlyLoopHostConfig {
                max_messages: 8,
                require_model_route_snapshot: false,
            },
        )
    }

    fn request(&self) -> RebornLoopDriverHostRequest {
        RebornLoopDriverHostRequest {
            claimed_run: self.claimed.clone(),
            loop_run_context: self.context.clone(),
        }
    }
}

fn invocation(
    surface_version: &CapabilitySurfaceVersion,
    capability_id: &str,
) -> CapabilityInvocation {
    CapabilityInvocation {
        surface_version: surface_version.clone(),
        capability_id: CapabilityId::new(capability_id).expect("capability id literal is valid"),
        input_ref: CapabilityInputRef::new(format!("input:{capability_id}"))
            .expect("input ref literal is valid"),
    }
}

fn expect_denied_with(outcome: CapabilityOutcome, expected_kind: &str) {
    match outcome {
        CapabilityOutcome::Denied(denied) => {
            assert_eq!(
                denied.reason_kind,
                CapabilityDeniedReasonKind::unknown(expected_kind)
                    .expect("expected reason kind literal is valid"),
                "denied reason_kind did not match"
            );
        }
        other => panic!("expected CapabilityOutcome::Denied, got {other:?}"),
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn predicate_deny_hook_short_circuits_inner_port() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();

    let host = fixture
        .factory()
        .with_hook_dispatcher(predicate_deny_dispatcher())
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability returns a (denied) outcome, not an error");

    expect_denied_with(outcome, "hook_denied");
    assert!(
        inner.invocations().is_empty(),
        "inner port must NOT be invoked when a hook denies; got {:?}",
        inner.invocations()
    );
}

#[tokio::test]
async fn non_matching_invocation_passes_through_to_inner_port() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();

    // Privileged selective hook denies cap.blocked, allows everything else.
    let host = fixture
        .factory()
        .with_hook_dispatcher(selective_deny_dispatcher("cap.blocked"))
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.allowed"))
        .await
        .expect("invoke_capability succeeds for the allowed capability");

    assert!(
        matches!(outcome, CapabilityOutcome::Completed(_)),
        "non-matching hook decision must let the inner port complete the call; got {outcome:?}"
    );
    let invocations = inner.invocations();
    assert_eq!(
        invocations.len(),
        1,
        "inner port should have been invoked exactly once; got {invocations:?}"
    );
    assert_eq!(
        invocations[0].as_str(),
        "cap.allowed",
        "inner port invoked with wrong capability"
    );
}

#[tokio::test]
async fn hook_dispatch_emits_milestones_into_host_sink() {
    // Build a dispatcher with a run-scoped milestone sink attached *before*
    // wrapping in Arc (per the documented composition order). Verify that
    // hook activity surfaces in the host's milestone backend via the
    // RunScopedHookMilestoneSink adapter.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();

    let hook_id = HookId::for_builtin(
        "tests::hooks_integration::milestone_selective_deny",
        HookVersion::ONE,
    );
    let mut dispatcher = HookDispatcher::new(HookRegistry::new());
    dispatcher
        .install_builtin_before_capability(
            hook_id,
            HookPhase::Policy,
            Box::new(SelectiveDenyHook {
                target: "cap.blocked".to_string(),
            }),
        )
        .expect("install builtin gate hook");
    let hook_milestone_sink: Arc<RunScopedHookMilestoneSink> =
        Arc::new(RunScopedHookMilestoneSink::new(
            fixture.context.clone(),
            Arc::clone(&fixture.milestone_sink) as _,
        ));
    dispatcher = dispatcher.with_milestone_sink(hook_milestone_sink);
    let dispatcher = Arc::new(dispatcher);

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher + telemetry installed");

    let _ = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke returns an outcome");

    let milestones = fixture.milestone_sink.milestones();
    let mut saw_dispatched = false;
    let mut saw_deny_decision = false;
    for m in &milestones {
        match &m.kind {
            LoopHostMilestoneKind::HookDispatched { point, .. } if point == "before_capability" => {
                saw_dispatched = true;
            }
            LoopHostMilestoneKind::HookDecisionEmitted { decision, .. } => {
                if decision.kind_name() == "deny" {
                    saw_deny_decision = true;
                }
            }
            _ => {}
        }
    }
    assert!(
        saw_dispatched,
        "expected HookDispatched milestone in {milestones:?}"
    );
    assert!(
        saw_deny_decision,
        "expected deny decision milestone in {milestones:?}"
    );
}

#[tokio::test]
async fn factory_without_hook_dispatcher_reaches_inner_port_for_blocked_capability() {
    // Proves that the hook wiring is genuinely opt-in: the SAME capability
    // that gets denied with a dispatcher installed must reach the inner port
    // when no dispatcher is configured.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();

    let host = fixture
        .factory()
        // Note: no `.with_hook_dispatcher(...)` call here.
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds without hook dispatcher");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability succeeds without hooks");

    assert!(
        matches!(outcome, CapabilityOutcome::Completed(_)),
        "without a dispatcher, the inner port must complete the call; got {outcome:?}"
    );
    let invocations = inner.invocations();
    assert_eq!(invocations.len(), 1, "inner port invoked exactly once");
    assert_eq!(invocations[0].as_str(), "cap.blocked");
}

#[tokio::test]
async fn pause_approval_hook_surfaces_as_approval_required_with_real_gate_ref() {
    // Proves that PauseApproval decisions no longer fall through to the
    // degraded `Denied` mapping. The middleware uses the default
    // `UuidHookGateRefFactory` to mint a real, validated `LoopGateRef` and
    // surfaces the hook intent as `CapabilityOutcome::ApprovalRequired`.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();

    let host = fixture
        .factory()
        .with_hook_dispatcher(pause_approval_dispatcher())
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability returns a (suspended) outcome, not an error");

    match outcome {
        CapabilityOutcome::ApprovalRequired {
            gate_ref,
            safe_summary,
        } => {
            assert!(
                gate_ref.as_str().starts_with("gate:hook-approval-"),
                "gate ref does not match expected prefix: {}",
                gate_ref.as_str()
            );
            assert_eq!(safe_summary, "integration-test pause approval");
        }
        other => panic!("expected ApprovalRequired, got {other:?}"),
    }
    assert!(
        inner.invocations().is_empty(),
        "inner port must NOT be invoked when a hook pauses; got {:?}",
        inner.invocations()
    );
}
