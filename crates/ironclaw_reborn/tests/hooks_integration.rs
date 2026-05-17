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
use ironclaw_hooks::dispatch::{HookDispatcher, HookDispatcherBuilder};
use ironclaw_hooks::evaluator::PredicateEvaluator;
use ironclaw_hooks::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
use ironclaw_hooks::installed_hook::PredicateBackedBeforeCapabilityHook;
use ironclaw_hooks::kinds::observer::NoteCategory;
use ironclaw_hooks::ordering::HookPhase;
use ironclaw_hooks::points::{BeforeCapabilityHookContext, ObserverHookContext};
use ironclaw_hooks::predicate::{
    CapabilityPredicate, HookPredicateSpec, OnExceededAction, ValueOrRateBound,
};
use ironclaw_hooks::registry::{HookBindingScope, HookPointSpec, HookRegistry};
use ironclaw_hooks::sink::{
    ObserverHook, ObserverSink, PrivilegedBeforeCapabilityHook, PrivilegedGateSink,
    RestrictedBeforeCapabilityHook, RestrictedGateSink,
};
use ironclaw_host_api::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    AlwaysAliveRunCancellationFactory, HostManagedModelError, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_reborn::{
    LoopCapabilityInputResolver, RebornLoopDriverHostFactory, RebornLoopDriverHostRequest,
    TextOnlyLoopHostConfig,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CheckpointStateStore, EventCursor, InMemoryCheckpointStateStore,
    InMemoryLoopCheckpointStore, InMemoryRunProfileResolver, InMemoryTurnStateStore, LoopResultRef,
    PutCheckpointStateRequest, ReplyTargetBindingRef, RunProfileId, RunProfileResolutionRequest,
    RunProfileResolver, RunProfileVersion, SourceBindingRef, TurnLeaseToken, TurnRunId,
    TurnRunnerId, TurnScope, TurnStatus,
    run_profile::{
        AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome,
        CapabilityDeniedReasonKind, CapabilityDescriptorView, CapabilityInputRef,
        CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage, CapabilitySurfaceVersion,
        InMemoryLoopHostMilestoneSink, LoopCapabilityPort, LoopCheckpointKind, LoopCheckpointPort,
        LoopCheckpointRequest, LoopHostMilestoneKind, LoopModelPort, LoopModelRequest,
        LoopPromptPort, LoopRunContext, LoopTranscriptPort, RunScopedHookMilestoneSink,
        VisibleCapabilityRequest, VisibleCapabilitySurface,
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
            terminate_hint: false,
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

/// Capability port whose surface includes per-capability provider info.
/// Used by the OwnCapabilities-scope tests to drive the provider-resolver
/// path (henrypark133 Critical #2).
struct ProviderAwareCapabilityPort {
    invocations: Mutex<Vec<CapabilityId>>,
    surface_version: CapabilitySurfaceVersion,
    descriptors: Vec<CapabilityDescriptorView>,
}

impl ProviderAwareCapabilityPort {
    fn new(descriptors: Vec<CapabilityDescriptorView>) -> Self {
        Self {
            invocations: Mutex::new(Vec::new()),
            surface_version: CapabilitySurfaceVersion::new("hooks-integration:v1")
                .expect("surface version literal is valid"),
            descriptors,
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
impl LoopCapabilityPort for ProviderAwareCapabilityPort {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            version: self.surface_version.clone(),
            descriptors: self.descriptors.clone(),
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
            terminate_hint: false,
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
    descriptor_with_provider(capability_id, None)
}

fn descriptor_with_provider(
    capability_id: &str,
    provider: Option<ironclaw_host_api::ExtensionId>,
) -> CapabilityDescriptorView {
    CapabilityDescriptorView {
        capability_id: CapabilityId::new(capability_id).expect("capability id literal is valid"),
        provider,
        runtime: ironclaw_host_api::RuntimeKind::Wasm,
        safe_name: capability_id.to_string(),
        safe_description: format!("test capability {capability_id}"),
        concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::Exclusive,
    }
}

// ─── Model-gateway stub ────────────────────────────────────────────────────

/// Minimal `HostManagedModelGateway` stub. Most integration tests don't drive
/// the model port — the gateway is only required because the factory's type
/// signature demands one. The observer-middleware tests (`observer_hook_*`)
/// do drive `stream_model`, so the gateway returns a successful assistant
/// reply rather than panicking. Capability-port tests still pass `cap.allowed`
/// / `cap.blocked` through the capability seam without ever invoking the
/// model gateway.
struct UnusedGateway;

#[async_trait]
impl HostManagedModelGateway for UnusedGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply(
            "integration-test stub reply",
        ))
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

/// Privileged builtin hook that panics on every invocation. Used to drive
/// slot-poisoning in the dispatcher so we can prove that fresh dispatchers
/// per host build do not inherit poisoning from an earlier run.
struct PanickingHook;

#[async_trait]
impl PrivilegedBeforeCapabilityHook for PanickingHook {
    async fn evaluate(
        &self,
        _ctx: &BeforeCapabilityHookContext,
        _sink: &mut dyn PrivilegedGateSink,
    ) {
        panic!("panicking hook for isolation regression test");
    }
}

fn panicking_dispatcher() -> Arc<HookDispatcher> {
    let hook_id = HookId::for_builtin("tests::hooks_integration::panicking_hook", HookVersion::ONE);
    HookDispatcherBuilder::new(HookRegistry::new())
        .install_builtin_before_capability(hook_id, HookPhase::Policy, Box::new(PanickingHook))
        .expect("install panicking hook")
        .build_arc()
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
    HookDispatcherBuilder::new(HookRegistry::new())
        .install_installed_before_capability(
            hook_id,
            HookPhase::Policy,
            ironclaw_host_api::ExtensionId::new("integration-tests").expect("valid ext id"),
            HookBindingScope::Global,
            Box::new(PauseApprovalHook),
        )
        .expect("install pause-approval hook")
        .build_arc()
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

    HookDispatcherBuilder::new(HookRegistry::new())
        .install_installed_before_capability(
            hook_id,
            HookPhase::Policy,
            ironclaw_host_api::ExtensionId::new("integration-tests").expect("valid ext id"),
            HookBindingScope::Global,
            Box::new(hook),
        )
        .expect("Installed-tier predicate hook installs at policy phase")
        .build_arc()
}

fn selective_deny_dispatcher(target: &str) -> Arc<HookDispatcher> {
    // SelectiveDenyHook is a Privileged (Builtin-tier) hook so it may mint
    // .allow() — which is exactly what we need to prove pass-through.
    let hook_id = HookId::for_builtin("tests::hooks_integration::selective_deny", HookVersion::ONE);
    let hook = SelectiveDenyHook {
        target: target.to_string(),
    };
    HookDispatcherBuilder::new(HookRegistry::new())
        .install_builtin_before_capability(hook_id, HookPhase::Policy, Box::new(hook))
        .expect("Builtin-tier hook installs at policy phase")
        .build_arc()
}

// ─── Fixture for building hosts with the factory ───────────────────────────

struct Fixture {
    thread_service: Arc<InMemorySessionThreadService>,
    checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    turn_state_store: Arc<InMemoryTurnStateStore>,
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
        let turn_state_store = Arc::new(InMemoryTurnStateStore::default());
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
            turn_state_store,
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
            Arc::clone(&self.turn_state_store) as _,
            Arc::clone(&self.loop_checkpoint_store) as _,
            Arc::clone(&self.milestone_sink) as _,
            TextOnlyLoopHostConfig {
                max_messages: 8,
                require_model_route_snapshot: false,
            },
        )
        .with_cancellation_factory(Arc::new(AlwaysAliveRunCancellationFactory))
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

    // Exercises the new factory-closure path: a fresh dispatcher is minted
    // for this single host build. The other tests in this file still pin the
    // legacy `with_hook_dispatcher(Arc<HookDispatcher>)` adapter, so the
    // backward-compat shape stays covered as well.
    let host = fixture
        .factory()
        .with_hook_dispatcher_factory(predicate_deny_dispatcher)
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
    let hook_milestone_sink: Arc<RunScopedHookMilestoneSink> =
        Arc::new(RunScopedHookMilestoneSink::new(
            fixture.context.clone(),
            Arc::clone(&fixture.milestone_sink) as _,
        ));
    let dispatcher = HookDispatcherBuilder::new(HookRegistry::new())
        .with_milestone_sink(hook_milestone_sink)
        .install_builtin_before_capability(
            hook_id,
            HookPhase::Policy,
            Box::new(SelectiveDenyHook {
                target: "cap.blocked".to_string(),
            }),
        )
        .expect("install builtin gate hook")
        .build_arc();

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
            LoopHostMilestoneKind::HookDecisionEmitted { decision, .. }
                if decision.kind_name() == "deny" =>
            {
                saw_deny_decision = true;
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
async fn per_build_dispatcher_state_does_not_leak_across_runs() {
    // Regression for codex C2: dispatcher-owned mutable state (slot
    // poisoning, in particular) must not survive across host builds when the
    // factory-closure path is used. We install a panicking hook, build two
    // hosts back-to-back, invoke each, and check that build 2 still actually
    // *dispatched* the hook — i.e., it didn't inherit a poisoned slot from
    // build 1.
    let fixture = Fixture::new().await;

    // Counter proves the closure was called once per build.
    let build_count = Arc::new(Mutex::new(0usize));
    let build_count_for_closure = Arc::clone(&build_count);

    let closure_context = fixture.context.clone();
    let closure_milestone_sink = Arc::clone(&fixture.milestone_sink);
    let factory = fixture.factory().with_hook_dispatcher_factory(move || {
        *build_count_for_closure
            .lock()
            .expect("build counter mutex not poisoned") += 1;
        // Fresh dispatcher every call — no shared poison state.
        let hook_id = HookId::for_builtin(
            "tests::hooks_integration::panicking_hook_per_build",
            HookVersion::ONE,
        );
        let sink: Arc<RunScopedHookMilestoneSink> = Arc::new(RunScopedHookMilestoneSink::new(
            closure_context.clone(),
            Arc::clone(&closure_milestone_sink) as _,
        ));
        HookDispatcherBuilder::new(HookRegistry::new())
            .with_milestone_sink(sink)
            .install_builtin_before_capability(hook_id, HookPhase::Policy, Box::new(PanickingHook))
            .expect("install panicking hook")
            .build_arc()
    });

    let surface_version = fixture.surface_version.clone();

    // Build 1: dispatch panics, slot poisoned in *that* dispatcher.
    let inner_one = Arc::new(RecordingCapabilityPort::new());
    let host_one = factory
        .build_text_only_host_with_capabilities(fixture.request(), inner_one.clone())
        .await
        .expect("first host builds");
    let _ = host_one
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke returns an outcome");

    // Build 2: fresh dispatcher, hook should NOT be inherited as poisoned.
    let inner_two = Arc::new(RecordingCapabilityPort::new());
    let host_two = factory
        .build_text_only_host_with_capabilities(fixture.request(), inner_two.clone())
        .await
        .expect("second host builds");
    let _ = host_two
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke returns an outcome");

    assert_eq!(
        *build_count
            .lock()
            .expect("build counter mutex not poisoned"),
        2,
        "factory closure must be invoked exactly once per build"
    );

    // If state had leaked across builds, build 2 would have inherited the
    // slot poisoned by build 1 and skipped dispatch entirely — the panic
    // would happen once and the inner port would then be reached on build 2
    // (poisoned slot → no deny). With per-build dispatchers, each build gets
    // a fresh, un-poisoned slot, so the hook actually runs (and panics) on
    // every build, and the inner port is NEVER reached.
    assert!(
        inner_one.invocations().is_empty(),
        "build 1: inner port must not be invoked when hook panics fail-closed"
    );
    assert!(
        inner_two.invocations().is_empty(),
        "build 2: with a fresh dispatcher, the hook still runs and still \
         fails closed, so inner must not be invoked. If you see inner \
         invocations here, poison state leaked from build 1's dispatcher \
         into build 2."
    );

    // Milestones corroborate: each build emits its own HookDispatched +
    // HookFailed (two of each across the run).
    let milestones = fixture.milestone_sink.milestones();
    let dispatched_count = milestones
        .iter()
        .filter(|m| {
            matches!(
                &m.kind,
                LoopHostMilestoneKind::HookDispatched { point, .. } if point == "before_capability"
            )
        })
        .count();
    assert_eq!(
        dispatched_count, 2,
        "expected one HookDispatched per build; saw {dispatched_count}"
    );

    let failed_count = milestones
        .iter()
        .filter(|m| matches!(&m.kind, LoopHostMilestoneKind::HookFailed { .. }))
        .count();
    assert_eq!(
        failed_count, 2,
        "expected one HookFailed per build (per-build poisoning); saw {failed_count}"
    );
}

#[tokio::test]
async fn legacy_with_hook_dispatcher_shares_state_across_builds() {
    // Documents (and pins) the legacy back-compat semantic: when callers use
    // `with_hook_dispatcher(Arc<HookDispatcher>)`, all builds share one
    // dispatcher and therefore share poison state. This is the behavior the
    // codex C2 follow-up explicitly does NOT change for existing callers —
    // we keep the shape so old wiring still works, but new code should use
    // `with_hook_dispatcher_factory`.
    let fixture = Fixture::new().await;
    let dispatcher = panicking_dispatcher();
    let factory = fixture
        .factory()
        .with_hook_dispatcher(Arc::clone(&dispatcher));
    let surface_version = fixture.surface_version.clone();

    let inner_one = Arc::new(RecordingCapabilityPort::new());
    let host_one = factory
        .build_text_only_host_with_capabilities(fixture.request(), inner_one.clone())
        .await
        .expect("first host builds");
    let _ = host_one
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke returns outcome");

    let inner_two = Arc::new(RecordingCapabilityPort::new());
    let host_two = factory
        .build_text_only_host_with_capabilities(fixture.request(), inner_two.clone())
        .await
        .expect("second host builds");
    let _ = host_two
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke returns outcome");

    // Build 1: hook runs, panics, dispatcher fail-closes -> inner NOT
    // invoked, and the (shared) dispatcher poisons the slot for the rest of
    // its lifetime.
    assert!(
        inner_one.invocations().is_empty(),
        "build 1: inner not invoked (hook fail-closed on panic)"
    );
    // Build 2: same Arc<HookDispatcher> -> slot still poisoned -> hook is
    // skipped entirely -> composed decision is Allow -> inner IS invoked.
    // This is the legacy semantic that motivated the per-build factory: a
    // single bad run permanently disables the hook for every subsequent
    // build that shares the dispatcher.
    assert_eq!(
        inner_two.invocations().len(),
        1,
        "build 2 must reach the inner port via the shared+poisoned slot"
    );
}

#[tokio::test]
async fn pause_approval_hook_surfaces_as_approval_required_with_real_gate_ref() {
    // Proves that PauseApproval decisions can surface as `ApprovalRequired`
    // when a gate-ref factory is wired. The factory's default is fail-
    // closed (henrypark133 Critical #3 — refuse to mint a syntactically-
    // valid but router-unregistered ref); tests must opt into the dev-only
    // `UuidHookGateRefFactory` to exercise the affirmative path.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();

    let host = fixture
        .factory()
        .with_hook_dispatcher(pause_approval_dispatcher())
        .with_hook_gate_ref_factory(Arc::new(ironclaw_hooks::middleware::UuidHookGateRefFactory))
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

/// henrypark133 Critical #3 regression: with no gate-ref factory wired,
/// a `PauseApproval` hook surfaces as `Denied`, not as `ApprovalRequired`
/// with an unresolvable ref. The default middleware factory is fail-closed
/// (`FailClosedHookGateRefFactory`) precisely so a hook can't park the
/// loop on a ref the host's approval gateway has never heard of.
#[tokio::test]
async fn pause_approval_with_default_factory_fails_closed_as_denied() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();

    let host = fixture
        .factory()
        .with_hook_dispatcher(pause_approval_dispatcher())
        // Deliberately NOT calling `with_hook_gate_ref_factory(...)` — the
        // default behavior must fail-closed.
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability returns a (denied) outcome, not an error");

    match outcome {
        CapabilityOutcome::Denied(_) => {} // expected
        other => {
            panic!("expected Denied (fail-closed) without a gate-ref factory wired; got {other:?}")
        }
    }
    assert!(
        inner.invocations().is_empty(),
        "inner port must NOT be invoked when a hook pauses; got {:?}",
        inner.invocations()
    );
}

// ─── Observer middleware integration tests ─────────────────────────────────
//
// These prove that `RebornLoopDriverHostFactory` wraps the model, transcript,
// and checkpoint ports with the observer middleware from
// `ironclaw_hooks::middleware::{model_port, transcript_port, checkpoint_port}`
// when a `HookDispatcher` is configured. Unit tests on the observer wrappers
// alone do not catch a factory regression — these do.

/// Builtin observer hook that counts invocations into a shared `Mutex`.
struct CountingObserver {
    seen: Arc<Mutex<u32>>,
}

#[async_trait]
impl ObserverHook for CountingObserver {
    async fn observe(&self, _ctx: &ObserverHookContext, sink: &mut dyn ObserverSink) {
        *self.seen.lock().expect("observer counter not poisoned") += 1;
        sink.note(NoteCategory::HookFired, "observer fired");
    }
}

/// Builtin observer that always panics — used to prove the outer call still
/// returns `Ok` and that the dispatcher records the failure via milestone.
struct PanickingObserver;

#[async_trait]
impl ObserverHook for PanickingObserver {
    async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
        panic!("intentional observer panic");
    }
}

fn observer_dispatcher_at(point: HookPointSpec, seen: Arc<Mutex<u32>>) -> Arc<HookDispatcher> {
    let hook_id = HookId::for_builtin(
        match point {
            HookPointSpec::AfterModel => "tests::hooks_integration::after_model_observer",
            HookPointSpec::AfterCapability => "tests::hooks_integration::after_capability_observer",
            HookPointSpec::AfterCheckpoint => "tests::hooks_integration::after_checkpoint_observer",
            other => panic!("unsupported observer point in test: {other:?}"),
        },
        HookVersion::ONE,
    );
    HookDispatcherBuilder::new(HookRegistry::new())
        .install_builtin_observer(
            hook_id,
            HookPhase::Telemetry,
            point,
            Box::new(CountingObserver { seen }),
        )
        .expect("install builtin observer")
        .build_arc()
}

#[tokio::test]
async fn after_model_fires_exactly_once_at_durable_boundary() {
    // henrypark133 Concerning #5 regression: previously, both the model
    // port and the transcript port dispatched `AfterModel`, so one
    // model exchange yielded two observer events — and the model-port
    // event fired *before* the assistant reply was durable. Now AfterModel
    // fires only from the transcript port's `finalize_assistant_message`,
    // i.e. the post-durable boundary. Drive `stream_model` (should NOT
    // fire) followed by `finalize_assistant_message` (SHOULD), and
    // assert the counter advances by exactly one.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let seen = Arc::new(Mutex::new(0u32));

    let host = fixture
        .factory()
        .with_hook_dispatcher(observer_dispatcher_at(
            HookPointSpec::AfterModel,
            Arc::clone(&seen),
        ))
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with AfterModel observer installed");

    // Base now requires `build_prompt_bundle` to pre-authorize each
    // `stream_model` call ("model request has no host-built prompt
    // bundle"). Build the bundle first so the authority is registered;
    // the bundle ref is referenced from `LoopModelRequest::messages`
    // (empty here is fine because no inline messages are needed).
    let bundle = host
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: vec![],
        })
        .await
        .expect("build_prompt_bundle succeeds before stream_model");
    host.stream_model(LoopModelRequest {
        messages: bundle.messages.clone(),
        surface_version: None,
        model_preference: None,
    })
    .await
    .expect("stream_model returns Ok via the wrapped model port");
    assert_eq!(
        *seen.lock().expect("observer counter not poisoned"),
        0,
        "AfterModel must NOT fire from stream_model — the model port \
         wrapper is a no-op for observers (the assistant reply is not \
         yet durable at that boundary)"
    );

    host.finalize_assistant_message(ironclaw_turns::run_profile::FinalizeAssistantMessage {
        reply: ironclaw_turns::run_profile::AssistantReply {
            content: "exactly-once test reply".to_string(),
        },
    })
    .await
    .expect("finalize_assistant_message returns Ok via the wrapped transcript port");

    assert_eq!(
        *seen.lock().expect("observer counter not poisoned"),
        1,
        "AfterModel must fire exactly once after finalize_assistant_message — \
         the transcript port owns the durable boundary"
    );
}

#[tokio::test]
async fn observer_hook_fires_after_capability_through_factory() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let seen = Arc::new(Mutex::new(0u32));

    let host = fixture
        .factory()
        .with_hook_dispatcher(observer_dispatcher_at(
            HookPointSpec::AfterCapability,
            Arc::clone(&seen),
        ))
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with AfterCapability observer installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.allowed"))
        .await
        .expect("invoke_capability returns a (completed) outcome");

    assert!(
        matches!(outcome, CapabilityOutcome::Completed(_)),
        "capability must complete normally, got {outcome:?}"
    );
    assert_eq!(
        *seen.lock().expect("observer counter not poisoned"),
        1,
        "AfterCapability observer must fire exactly once after a successful \
         capability invocation"
    );
}

#[tokio::test]
async fn observer_hook_fires_after_checkpoint_through_factory() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let seen = Arc::new(Mutex::new(0u32));

    // The HostManagedLoopCheckpointPort requires a pre-existing checkpoint
    // state record under the run's scope before it will write a loop
    // checkpoint, so seed one up front.
    let state_record = fixture
        .checkpoint_state_store
        .put_checkpoint_state(PutCheckpointStateRequest::new(
            fixture.context.scope.clone(),
            fixture.context.turn_id,
            fixture.context.run_id,
            fixture.context.checkpoint_schema_id.clone(),
            fixture.context.checkpoint_schema_version,
            LoopCheckpointKind::BeforeModel,
            b"observer-test-checkpoint-payload".to_vec(),
        ))
        .await
        .expect("seed checkpoint state record");

    let host = fixture
        .factory()
        .with_hook_dispatcher(observer_dispatcher_at(
            HookPointSpec::AfterCheckpoint,
            Arc::clone(&seen),
        ))
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with AfterCheckpoint observer installed");

    host.checkpoint(LoopCheckpointRequest {
        kind: LoopCheckpointKind::BeforeModel,
        state_ref: state_record.state_ref,
    })
    .await
    .expect("checkpoint write succeeds through the wrapped checkpoint port");

    assert_eq!(
        *seen.lock().expect("observer counter not poisoned"),
        1,
        "AfterCheckpoint observer must fire exactly once after a successful \
         checkpoint write — proves the factory wraps the checkpoint port"
    );
}

#[tokio::test]
async fn observer_panic_does_not_fail_model_call() {
    // A panicking observer hook must fail isolated: the model call returns
    // Ok, and the dispatcher records a HookFailed milestone with the
    // observer's hook id. The poison side effect is also visible through
    // the milestone stream.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());

    // Wrap the panicking-observer dispatcher in a run-scoped milestone sink
    // so HookFailed lands in the host milestone backend.
    let hook_id = HookId::for_builtin(
        "tests::hooks_integration::panicking_observer",
        HookVersion::ONE,
    );
    let hook_milestone_sink: Arc<RunScopedHookMilestoneSink> =
        Arc::new(RunScopedHookMilestoneSink::new(
            fixture.context.clone(),
            Arc::clone(&fixture.milestone_sink) as _,
        ));
    let dispatcher = HookDispatcherBuilder::new(HookRegistry::new())
        .with_milestone_sink(hook_milestone_sink)
        .install_builtin_observer(
            hook_id,
            HookPhase::Telemetry,
            HookPointSpec::AfterModel,
            Box::new(PanickingObserver),
        )
        .expect("install panicking observer")
        .build_arc();

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with panicking observer installed");

    // Drive the post-durable boundary (finalize_assistant_message) since
    // AfterModel now fires from the transcript port. The panic happens
    // inside the observer dispatch — must NOT propagate into the outer
    // finalize call (henrypark133 Concerning #5 + observer-fail-isolated).
    let response = host
        .finalize_assistant_message(ironclaw_turns::run_profile::FinalizeAssistantMessage {
            reply: ironclaw_turns::run_profile::AssistantReply {
                content: "panicking observer test reply".to_string(),
            },
        })
        .await;
    assert!(
        response.is_ok(),
        "observer panic must NOT propagate into the outer finalize call; got {response:?}"
    );

    // The dispatcher emits a HookFailed milestone for the panicking observer;
    // proves the observer poisoning is recorded without affecting the outer
    // port outcome.
    let saw_failed = fixture
        .milestone_sink
        .milestones()
        .iter()
        .any(|m| matches!(m.kind, LoopHostMilestoneKind::HookFailed { .. }));
    assert!(
        saw_failed,
        "expected a HookFailed milestone after observer panic; milestones = {:?}",
        fixture.milestone_sink.milestones()
    );
}

// ─── NumericSum predicate against real inputs ──────────────────────────────

/// Stub `LoopCapabilityInputResolver` that always returns the same JSON body
/// for every input ref. The NumericSum predicate test wires this resolver
/// through `RebornLoopDriverHostFactory::with_capability_input_resolver` so
/// the hook framework sees real numeric input and the predicate can
/// accumulate across invocations.
struct ConstantJsonInputResolver {
    payload: serde_json::Value,
}

#[async_trait]
impl LoopCapabilityInputResolver for ConstantJsonInputResolver {
    async fn resolve_capability_input(
        &self,
        _run_context: &LoopRunContext,
        _input_ref: &CapabilityInputRef,
    ) -> Result<serde_json::Value, AgentLoopHostError> {
        Ok(self.payload.clone())
    }
}

fn numeric_sum_dispatcher() -> Arc<HookDispatcher> {
    // RateOrValueCap with NumericSum over a "amount" field. Two consecutive
    // invocations each carrying amount=50 will sum to 100, which is strictly
    // greater than the configured max of 99 — so the second invocation must
    // be denied. The first invocation (sum = 50) is below the cap and is
    // expected to pass through to the inner port.
    let hook_id = HookId::derive(
        &ExtensionId("integration-tests".to_string()),
        "0.0.1",
        &HookLocalId("numeric-sum-amount".to_string()),
        HookVersion::ONE,
    );
    let spec = HookPredicateSpec::RateOrValueCap {
        when: CapabilityPredicate::NameEquals {
            name: "cap.allowed".to_string(),
        },
        bound: ValueOrRateBound::NumericSum {
            max: "99".to_string(),
            field: "amount".to_string(),
            window: "24h".to_string(),
        },
        on_exceeded: OnExceededAction::Deny {
            reason: "numeric_sum_cap_exceeded".to_string(),
        },
    };
    let evaluator = Arc::new(PredicateEvaluator::new());
    let hook = PredicateBackedBeforeCapabilityHook::new(hook_id, spec, evaluator);

    HookDispatcherBuilder::new(HookRegistry::new())
        .install_installed_before_capability(
            hook_id,
            HookPhase::Policy,
            ironclaw_host_api::ExtensionId::new("integration-tests").expect("valid ext id"),
            HookBindingScope::Global,
            Box::new(hook),
        )
        .expect("Installed-tier predicate hook installs at policy phase")
        .build_arc()
}

#[tokio::test]
async fn numeric_sum_predicate_caps_total_value_against_real_inputs() {
    // Proves the production wiring: with both a `HookDispatcher` AND a
    // capability input resolver installed on the factory, NumericSum
    // predicates evaluate against real, sanitized capability arguments.
    // Without the resolver, the predicate would have failed closed on the
    // first call (the framework's default NullCapabilityInputResolver
    // returns None, which the evaluator treats as "unresolved" and denies).
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();

    let resolver: Arc<dyn LoopCapabilityInputResolver> = Arc::new(ConstantJsonInputResolver {
        payload: serde_json::json!({"amount": "50"}),
    });

    let host = fixture
        .factory()
        .with_hook_dispatcher(numeric_sum_dispatcher())
        .with_capability_input_resolver(resolver)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher + capability input resolver installed");

    // First invocation: cumulative sum = 50, below the cap of 99 → allowed.
    let first = host
        .invoke_capability(invocation(&surface_version, "cap.allowed"))
        .await
        .expect("first invocation completes successfully");
    assert!(
        matches!(first, CapabilityOutcome::Completed(_)),
        "first invocation must pass through to inner port; got {first:?}"
    );

    // Second invocation: cumulative sum = 100 (> 99) → denied by hook.
    let second = host
        .invoke_capability(invocation(&surface_version, "cap.allowed"))
        .await
        .expect("second invocation returns an outcome, not an error");
    expect_denied_with(second, "hook_denied");

    // Inner port was reached exactly once (the first call); the second call
    // was short-circuited at the hook seam.
    let invocations = inner.invocations();
    assert_eq!(
        invocations.len(),
        1,
        "inner port must have been invoked only for the first (under-cap) call; got {invocations:?}"
    );
    assert_eq!(invocations[0].as_str(), "cap.allowed");
}

/// C3 regression: a deny hook authored by ext-A and scoped to
/// `OwnCapabilities` must NOT intercept invocations whose provider is unknown
/// (or belongs to a different extension). The conservative default for an
/// unresolved provider is "do not fire", so the inner port runs and completes
/// the call normally — proving manifest-declared scope is enforced at
/// dispatch time, not just parsed at install.
#[tokio::test]
async fn installed_hook_with_own_scope_does_not_fire_on_other_provider_capabilities() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();

    // Build a dispatcher with an Installed-tier always-deny hook authored by
    // ext-A and scoped to OwnCapabilities. With the default null provider
    // resolver in the factory, every invocation surfaces as
    // `ctx.provider == None`, which never satisfies OwnCapabilities.
    let hook_id = HookId::derive(
        &ExtensionId("ext-a".to_string()),
        "0.0.1",
        &HookLocalId("c3-own-scope-deny".to_string()),
        HookVersion::ONE,
    );
    struct AlwaysDeny;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for AlwaysDeny {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.deny("c3-own-scope-deny-fired");
        }
    }
    let dispatcher = HookDispatcherBuilder::new(HookRegistry::new())
        .install_installed_before_capability(
            hook_id,
            HookPhase::Policy,
            ironclaw_host_api::ExtensionId::new("ext-a").expect("valid ext id"),
            HookBindingScope::OwnCapabilities,
            Box::new(AlwaysDeny),
        )
        .expect("install installed hook with own-scope")
        .build_arc();

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability returns an outcome");

    assert!(
        matches!(outcome, CapabilityOutcome::Completed(_)),
        "OwnCapabilities-scoped ext-A hook must not fire when the provider \
         is unknown; the inner port must complete the call. Got {outcome:?}"
    );
    let invocations = inner.invocations();
    assert_eq!(
        invocations.len(),
        1,
        "inner port should have been invoked exactly once; got {invocations:?}"
    );
    assert_eq!(invocations[0].as_str(), "cap.blocked");
}

// ─── henrypark133 Critical #2: OwnCapabilities provider resolver ──────────

/// Build a dispatcher with an Installed-tier always-deny hook authored by
/// `owning_ext`, scoped to `OwnCapabilities`.
fn own_capabilities_dispatcher(owning_ext: &str, local_id: &str) -> Arc<HookDispatcher> {
    let hook_id = HookId::derive(
        &ExtensionId(owning_ext.to_string()),
        "0.0.1",
        &HookLocalId(local_id.to_string()),
        HookVersion::ONE,
    );
    struct AlwaysDeny;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for AlwaysDeny {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.deny("own-scope-deny-fired");
        }
    }
    HookDispatcherBuilder::new(HookRegistry::new())
        .install_installed_before_capability(
            hook_id,
            HookPhase::Policy,
            ironclaw_host_api::ExtensionId::new(owning_ext).expect("valid ext id"),
            HookBindingScope::OwnCapabilities,
            Box::new(AlwaysDeny),
        )
        .expect("install installed hook with own-scope")
        .build_arc()
}

/// Positive case: hook owned by ext-a, capability has provider=ext-a.
/// With the new surface-backed provider resolver wired by the factory,
/// `ctx.provider == Some(ext-a)` matches the binding's `owning_extension`,
/// so the OwnCapabilities filter permits the hook and the deny fires.
#[tokio::test]
async fn own_capabilities_hook_fires_when_provider_matches() {
    let fixture = Fixture::new().await;
    let ext_a = ironclaw_host_api::ExtensionId::new("ext-a").expect("valid ext id");
    let inner = Arc::new(ProviderAwareCapabilityPort::new(vec![
        descriptor_with_provider("cap.alpha", Some(ext_a.clone())),
    ]));
    let surface_version = CapabilitySurfaceVersion::new("hooks-integration:v1").expect("ok");

    let host = fixture
        .factory()
        .with_hook_dispatcher(own_capabilities_dispatcher("ext-a", "cap-a-own-deny"))
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.alpha"))
        .await
        .expect("invoke returns an outcome");

    match outcome {
        CapabilityOutcome::Denied(_) => {} // expected: hook fired
        other => panic!(
            "OwnCapabilities hook must fire when provider matches the binding's owning_extension; got {other:?}"
        ),
    }
    assert!(
        inner.invocations().is_empty(),
        "inner port must NOT be invoked when the hook denies; got {:?}",
        inner.invocations()
    );
}

/// Negative case: hook owned by ext-a, capability has provider=ext-b.
/// The OwnCapabilities filter rejects this combination; the inner port
/// completes the call normally.
#[tokio::test]
async fn own_capabilities_hook_does_not_fire_when_provider_differs() {
    let fixture = Fixture::new().await;
    let ext_b = ironclaw_host_api::ExtensionId::new("ext-b").expect("valid ext id");
    let inner = Arc::new(ProviderAwareCapabilityPort::new(vec![
        descriptor_with_provider("cap.beta", Some(ext_b)),
    ]));
    let surface_version = CapabilitySurfaceVersion::new("hooks-integration:v1").expect("ok");

    let host = fixture
        .factory()
        .with_hook_dispatcher(own_capabilities_dispatcher("ext-a", "cap-a-foreign-deny"))
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.beta"))
        .await
        .expect("invoke returns an outcome");

    assert!(
        matches!(outcome, CapabilityOutcome::Completed(_)),
        "ext-A's OwnCapabilities hook must NOT fire against ext-B's capability; got {outcome:?}"
    );
    assert_eq!(inner.invocations().len(), 1, "inner port should be invoked");
}

/// Unresolved-provider case: capability has provider=None. The
/// `OwnCapabilities` filter is conservative — hook does NOT fire when the
/// provider is unknown. This is the documented behavior from C3.
#[tokio::test]
async fn own_capabilities_hook_does_not_fire_when_provider_unknown() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(ProviderAwareCapabilityPort::new(vec![
        descriptor_with_provider("cap.unattributed", None),
    ]));
    let surface_version = CapabilitySurfaceVersion::new("hooks-integration:v1").expect("ok");

    let host = fixture
        .factory()
        .with_hook_dispatcher(own_capabilities_dispatcher(
            "ext-a",
            "cap-a-unresolved-deny",
        ))
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.unattributed"))
        .await
        .expect("invoke returns an outcome");

    assert!(
        matches!(outcome, CapabilityOutcome::Completed(_)),
        "OwnCapabilities must NOT fire when provider is unknown; got {outcome:?}"
    );
}

// ─── henrypark133 Critical #4: per-run hook telemetry attribution ─────────

/// Two-run telemetry test: build the factory once with the same builder
/// factory closure, drive a hook dispatch under run 1, then build a second
/// host with a fresh `LoopRunContext` and drive the same hook again. Both
/// runs share the dispatcher *builder* (so the closure is invoked twice,
/// minting one dispatcher per run), but the host factory attaches a
/// `RunScopedHookMilestoneSink` keyed to the *current* run context inside
/// `build_text_only_host_with_capabilities`. The test asserts the
/// milestones emitted in run 1 carry run 1's `run_id`, and the milestones
/// emitted in run 2 carry run 2's `run_id` — never the stale captured one
/// (henrypark133 Critical #4).
#[tokio::test]
async fn hook_telemetry_attribution_is_per_run_not_captured() {
    let fixture = Fixture::new().await;
    let inner_a = Arc::new(RecordingCapabilityPort::new());
    let inner_b = Arc::new(RecordingCapabilityPort::new());

    // Same dispatcher-builder closure used for both builds; if the factory
    // were capturing run_context inside the closure (the broken pattern),
    // run 2 would emit milestones under run 1's id.
    let factory_with_hook = fixture.factory().with_hook_dispatcher_builder_factory(|| {
        use ironclaw_hooks::dispatch::HookDispatcherBuilder as HDBuilder;
        use ironclaw_hooks::registry::HookRegistry as HReg;
        let hook_id = HookId::derive(
            &ExtensionId("ext-tele".to_string()),
            "0.0.1",
            &HookLocalId("deny-everything".to_string()),
            HookVersion::ONE,
        );
        struct AlwaysDeny;
        #[async_trait]
        impl RestrictedBeforeCapabilityHook for AlwaysDeny {
            async fn evaluate(
                &self,
                _ctx: &BeforeCapabilityHookContext,
                sink: &mut dyn RestrictedGateSink,
            ) {
                sink.deny("two-run-telemetry-test");
            }
        }
        HDBuilder::new(HReg::new())
            .install_installed_before_capability(
                hook_id,
                HookPhase::Policy,
                ironclaw_host_api::ExtensionId::new("ext-tele").expect("valid ext id"),
                HookBindingScope::Global,
                Box::new(AlwaysDeny),
            )
            .expect("install always-deny hook")
    });

    // Run 1.
    let request_1 = fixture.request();
    let run_id_1 = request_1.loop_run_context.run_id;
    let host_1 = factory_with_hook
        .build_text_only_host_with_capabilities(request_1, inner_a)
        .await
        .expect("host 1 builds");
    let _ = host_1
        .invoke_capability(invocation(&fixture.surface_version, "cap.x"))
        .await
        .expect("invoke 1 returns outcome");

    // Run 2: fresh turn_id + run_id, otherwise same fixture state.
    let mut request_2 = fixture.request();
    let new_turn_id = ironclaw_turns::TurnId::new();
    let new_run_id = TurnRunId::new();
    request_2.loop_run_context = LoopRunContext::new(
        request_2.loop_run_context.scope.clone(),
        new_turn_id,
        new_run_id,
        request_2.loop_run_context.resolved_run_profile.clone(),
    );
    request_2.claimed_run.state.turn_id = new_turn_id;
    request_2.claimed_run.state.run_id = new_run_id;
    let host_2 = factory_with_hook
        .build_text_only_host_with_capabilities(request_2, inner_b)
        .await
        .expect("host 2 builds with fresh run context");
    let _ = host_2
        .invoke_capability(invocation(&fixture.surface_version, "cap.x"))
        .await
        .expect("invoke 2 returns outcome");

    // Inspect the milestone sink. Hook milestones from run 1 must carry
    // run_id_1; hook milestones from run 2 must carry new_run_id. None of
    // them may carry a stale or swapped id.
    let milestones = fixture.milestone_sink.milestones();
    let hook_milestones: Vec<_> = milestones
        .iter()
        .filter(|m| {
            matches!(
                m.kind,
                LoopHostMilestoneKind::HookDispatched { .. }
                    | LoopHostMilestoneKind::HookDecisionEmitted { .. }
                    | LoopHostMilestoneKind::HookFailed { .. }
            )
        })
        .collect();
    assert!(
        !hook_milestones.is_empty(),
        "expected at least one hook milestone across the two runs"
    );

    let in_run_1: Vec<_> = hook_milestones
        .iter()
        .filter(|m| m.run_id == run_id_1)
        .collect();
    let in_run_2: Vec<_> = hook_milestones
        .iter()
        .filter(|m| m.run_id == new_run_id)
        .collect();
    let stale: Vec<_> = hook_milestones
        .iter()
        .filter(|m| m.run_id != run_id_1 && m.run_id != new_run_id)
        .collect();

    assert!(
        !in_run_1.is_empty(),
        "expected hook milestones tagged with run 1's id"
    );
    assert!(
        !in_run_2.is_empty(),
        "expected hook milestones tagged with run 2's id; the factory must \
         attach the run-scoped sink fresh per build, not reuse a captured one"
    );
    assert!(
        stale.is_empty(),
        "no milestone may carry a run id outside the two test runs; got {stale:?}"
    );
}

// ─── henrypark133 Critical #1: before_prompt hook resolver path ───────────

/// Drives the full path: install a `before_prompt` hook that emits an
/// envelope-wrapped snippet, build the prompt bundle through
/// `RebornLoopDriverHostFactory`, and verify that (a) the bundle includes
/// a synthetic `msg:hook.*` ref and (b) the build did NOT fail closed
/// (which it would if the factory neglected to wire the materialization
/// sink). The sink-wired path also writes the safe content into the
/// `InstructionMaterializationStore` so the downstream model resolver
/// can find it; that store write is what makes the ref resolvable.
#[tokio::test]
async fn before_prompt_hook_message_is_resolvable_via_factory_wiring() {
    use ironclaw_hooks::dispatch::HookDispatcherBuilder as HDBuilder;
    use ironclaw_hooks::registry::HookRegistry as HReg;
    use ironclaw_hooks::sink::{RestrictedBeforePromptHook, RestrictedMutatorSink};

    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());

    let hook_id = HookId::derive(
        &ExtensionId("ext-prompt".to_string()),
        "0.0.1",
        &HookLocalId("prompt-inject".to_string()),
        HookVersion::ONE,
    );

    struct InjectingHook;
    #[async_trait]
    impl RestrictedBeforePromptHook for InjectingHook {
        async fn evaluate(
            &self,
            _ctx: &ironclaw_hooks::points::BeforePromptHookContext,
            sink: &mut dyn RestrictedMutatorSink,
        ) {
            let _ = sink.add_envelope_snippet(
                "injected hook context".to_string(),
                ironclaw_hooks::kinds::mutator::PatchOrdinalHint::Last,
            );
        }
    }

    let dispatcher = HDBuilder::new(HReg::new())
        .install_installed_before_prompt(
            hook_id,
            HookPhase::Policy,
            ironclaw_host_api::ExtensionId::new("ext-prompt").expect("valid ext id"),
            HookBindingScope::Global,
            Box::new(InjectingHook),
        )
        .expect("install installed before_prompt hook")
        .build_arc();

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with before_prompt hook installed");

    let bundle = host
        .build_prompt_bundle(ironclaw_turns::run_profile::LoopPromptBundleRequest {
            mode: ironclaw_turns::run_profile::PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: vec![],
        })
        .await
        .expect(
            "build_prompt_bundle must succeed; if this errors with `materialization sink \
             is wired` the factory regressed (henrypark133 Critical #1)",
        );

    // The bundle should contain at least one hook-injected ref. Each hook
    // message uses the `msg:hook.<ordinal>.<hash>` convention.
    let hook_message_count = bundle
        .messages
        .iter()
        .filter(|m| m.content_ref.as_str().starts_with("msg:hook."))
        .count();
    assert!(
        hook_message_count >= 1,
        "expected at least one msg:hook.* ref in the prompt bundle; got {:?}",
        bundle.messages
    );
}
