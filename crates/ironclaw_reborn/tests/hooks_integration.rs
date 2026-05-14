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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_events::{
    DurableEventLog, DurableEventSink, EventCursor as RuntimeEventCursor, EventSink,
    EventStreamKey, InMemoryDurableEventLog, ReadScope, RuntimeEvent, RuntimeEventKind,
};
use ironclaw_hooks::HookRegistrar;
use ironclaw_hooks::dispatch::{HookDispatcher, HookDispatcherBuilder};
use ironclaw_hooks::error::HookError;
use ironclaw_hooks::evaluator::PredicateEvaluator;
use ironclaw_hooks::failure_policy::{FailureCategory, FailureDisposition};
use ironclaw_hooks::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
use ironclaw_hooks::installed_hook::PredicateBackedBeforeCapabilityHook;
use ironclaw_hooks::kinds::observer::NoteCategory;
use ironclaw_hooks::manifest::{
    HookManifestBody, HookManifestEntry, HookManifestKind, HookManifestScope, WasmBudget,
};
use ironclaw_hooks::ordering::HookPhase;
use ironclaw_hooks::points::{
    BeforeCapabilityHookContext, BeforePromptHookContext, EventTriggeredHookContext,
    ObserverHookContext,
};
use ironclaw_hooks::predicate::{
    CapabilityPredicate, HookPredicateSpec, OnExceededAction, ValueOrRateBound,
};
use ironclaw_hooks::registry::{HookBindingScope, HookPointSpec, HookRegistry};
use ironclaw_hooks::sink::{
    EventTriggeredHook, EventTriggeredObserverSink, ObserverHook, ObserverSink,
    PrivilegedBeforeCapabilityHook, PrivilegedGateSink, RestrictedBeforeCapabilityHook,
    RestrictedGateSink,
};
use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, ProjectId, ResourceScope, RuntimeKind, TenantId, ThreadId,
    UserId,
};
use ironclaw_hooks::wasm::{
    WasmHookModuleRequest, WasmHookModuleResolver, WasmHookRuntime, WasmHookRuntimeError,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse, LoopCapabilityInputResolver,
};
use ironclaw_reborn::hook_gate_refs::{
    HookGateActorBinding, HookGateReservationContext, HookGateResolutionRequest, HookGateRouter,
    InMemoryHookGateRouter, RouterBackedHookGateRefFactory, hook_gate_arguments_digest,
};
use ironclaw_reborn::loop_driver_host::{
    EventTriggeredHookSubscription, RebornLoopDriverHostFactory, RebornLoopDriverHostRequest,
    TextOnlyLoopHostConfig,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    SessionThreadService, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, CheckpointStateStore, EventCursor,
    GetRunStateRequest, InMemoryCheckpointStateStore, InMemoryRunProfileResolver,
    InMemoryTurnStateStore, LoopResultRef, PutCheckpointStateRequest, ReplyTargetBindingRef,
    ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileResolutionRequest,
    RunProfileResolver, RunProfileVersion, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnActor, TurnAdmissionPolicy, TurnError, TurnLeaseToken, TurnRunId, TurnRunState,
    TurnRunnerId, TurnScope, TurnStateStore, TurnStatus,
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

// ─── Static turn state store ──────────────────────────────────────────────
//
// The factory's cancellation handle builder looks up the run by id from
// the supplied `TurnStateStore`. `InMemoryTurnStateStore::default()` is
// empty, so we wrap the claimed state directly. This mirrors the
// `StaticTurnStateStore` pattern used by `tests/loop_driver_host.rs`.

struct StaticTurnStateStore {
    state: Mutex<TurnRunState>,
}

impl StaticTurnStateStore {
    fn new(state: TurnRunState) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }
}

#[async_trait]
impl TurnStateStore for StaticTurnStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn TurnAdmissionPolicy,
        _run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn should not be called by static test turn state store")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn should not be called by static test turn state store")
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        panic!("request_cancel should not be called by static test turn state store")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Ok(self
            .state
            .lock()
            .expect("static turn state lock not poisoned")
            .clone())
    }
}

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
        parameters_schema: serde_json::Value::Null,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenRuntimeEvent {
    cursor: RuntimeEventCursor,
    kind: RuntimeEventKind,
    provider: Option<ironclaw_host_api::ExtensionId>,
    hook_id: Option<String>,
}

struct RecordingEventTriggeredHook {
    seen: Arc<Mutex<Vec<SeenRuntimeEvent>>>,
    delay: Option<Duration>,
}

#[async_trait]
impl EventTriggeredHook for RecordingEventTriggeredHook {
    async fn observe(
        &self,
        ctx: &EventTriggeredHookContext<'_>,
        sink: &mut dyn EventTriggeredObserverSink,
    ) {
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        self.seen
            .lock()
            .expect("seen-event mutex not poisoned")
            .push(SeenRuntimeEvent {
                cursor: ctx.event_cursor,
                kind: ctx.event.kind,
                provider: ctx.event.provider.clone(),
                hook_id: ctx.event.hook_id.clone(),
            });
        sink.note(NoteCategory::HookFired, "event hook fired");
    }
}

fn event_triggered_dispatcher(
    event_kind: RuntimeEventKind,
    seen: Arc<Mutex<Vec<SeenRuntimeEvent>>>,
) -> Arc<HookDispatcher> {
    event_triggered_dispatcher_with_scope(
        event_kind,
        seen,
        HookBindingScope::Global,
        "integration-tests",
        None,
    )
}

fn event_triggered_dispatcher_with_scope(
    event_kind: RuntimeEventKind,
    seen: Arc<Mutex<Vec<SeenRuntimeEvent>>>,
    scope: HookBindingScope,
    owning_extension: &str,
    delay: Option<Duration>,
) -> Arc<HookDispatcher> {
    let hook_id = HookId::derive(
        &ExtensionId(owning_extension.to_string()),
        "0.0.1",
        &HookLocalId(format!("event-{event_kind:?}")),
        HookVersion::ONE,
    );
    HookDispatcherBuilder::new(HookRegistry::new())
        .with_timeout(Duration::from_millis(500))
        .install_installed_event_triggered(
            hook_id,
            HookPhase::Telemetry,
            event_kind,
            ironclaw_host_api::ExtensionId::new(owning_extension).expect("valid ext id"),
            scope,
            Box::new(RecordingEventTriggeredHook { seen, delay }),
        )
        .expect("event-triggered hook installs")
        .build_arc()
}

fn pause_approval_dispatcher() -> Arc<HookDispatcher> {
    let hook_id = HookId::derive(
        &ExtensionId::new("integration-tests").expect("valid ExtensionId in test"),
        "0.0.1",
        &HookLocalId::new("pause-approval").expect("valid HookLocalId in test"),
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
        &ExtensionId::new("integration-tests").expect("valid ExtensionId in test"),
        "0.0.1",
        &HookLocalId::new("deny-cap-blocked").expect("valid HookLocalId in test"),
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

#[derive(Default)]
struct InMemoryWasmHookModules {
    modules: Mutex<HashMap<String, Vec<u8>>>,
}

impl InMemoryWasmHookModules {
    fn with_wat(local_id: &str, wat_source: &str) -> Self {
        let mut modules = HashMap::new();
        modules.insert(
            local_id.to_string(),
            wat::parse_str(wat_source).expect("test wasm fixture must compile"),
        );
        Self {
            modules: Mutex::new(modules),
        }
    }
}

impl WasmHookModuleResolver for InMemoryWasmHookModules {
    fn resolve_module(
        &self,
        request: &WasmHookModuleRequest<'_>,
    ) -> Result<Vec<u8>, WasmHookRuntimeError> {
        self.modules
            .lock()
            .expect("wasm module resolver mutex not poisoned")
            .get(request.hook_local_id.as_str())
            .cloned()
            .ok_or_else(|| {
                WasmHookRuntimeError::module_unavailable(format!(
                    "missing fixture module for {}",
                    request.hook_local_id
                ))
            })
    }
}

fn wasm_dispatcher_from_wat(
    local_id: &str,
    kind: HookManifestKind,
    wat_source: &str,
    budget: WasmBudget,
) -> (Arc<HookDispatcher>, HookId) {
    wasm_dispatcher_from_wat_with_timeout(local_id, kind, wat_source, budget, None)
}

fn wasm_dispatcher_from_wat_with_timeout(
    local_id: &str,
    kind: HookManifestKind,
    wat_source: &str,
    budget: WasmBudget,
    dispatcher_timeout: Option<Duration>,
) -> (Arc<HookDispatcher>, HookId) {
    let resolver = Arc::new(InMemoryWasmHookModules::with_wat(local_id, wat_source));
    let runtime = Arc::new(WasmHookRuntime::new(resolver).expect("wasm runtime"));
    let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()))
        .with_wasm_runtime(runtime)
        .with_verified_grants(["integration-test-wasm-hooks".to_string()]);
    let body = HookManifestBody::Wasm {
        export: "evaluate".to_string(),
        budget,
    };
    let entry = HookManifestEntry::new(
        HookLocalId::new(local_id).expect("valid HookLocalId in test"),
        kind,
        body,
    )
    .with_scope(HookManifestScope::SameTenant)
    .with_requires_grant("integration-test-wasm-hooks");
    let mut builder = HookDispatcherBuilder::new(HookRegistry::new());
    if let Some(timeout) = dispatcher_timeout {
        builder = builder.with_timeout(timeout);
    }
    let (builder, ids) = registrar
        .install(
            ironclaw_host_api::ExtensionId::new("integration-tests").expect("valid ext id"),
            "0.0.1".to_string(),
            vec![entry],
            builder,
        )
        .expect("wasm hook installs");
    (builder.build_arc(), ids[0])
}

fn wasm_before_prompt_dispatcher_from_wat(
    local_id: &str,
    wat_source: &str,
    budget: WasmBudget,
) -> Arc<HookDispatcher> {
    let resolver = Arc::new(InMemoryWasmHookModules::with_wat(local_id, wat_source));
    let runtime = Arc::new(WasmHookRuntime::new(resolver).expect("wasm runtime"));
    let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()))
        .with_wasm_runtime(runtime)
        .with_verified_grants(["integration-test-wasm-hooks".to_string()]);
    let body = HookManifestBody::Wasm {
        export: "evaluate".to_string(),
        budget,
    };
    let entry = HookManifestEntry::new(
        HookLocalId::new(local_id).expect("valid HookLocalId in test"),
        HookManifestKind::BeforePrompt,
        body,
    )
    .with_scope(HookManifestScope::OwnCapabilities);
    let builder = HookDispatcherBuilder::new(HookRegistry::new());
    let (builder, _ids) = registrar
        .install(
            ironclaw_host_api::ExtensionId::new("integration-tests").expect("valid ext id"),
            "0.0.1".to_string(),
            vec![entry],
            builder,
        )
        .expect("wasm before_prompt hook installs");
    builder.build_arc()
}

const WASM_DENY_HOOK: &str = r#"
(module
  (import "ic:hooks/before-capability@1" "deny" (func $deny (param i32) (result i32)))
  (func (export "evaluate")
    i32.const 1
    call $deny
    drop)
)
"#;

const WASM_INFINITE_LOOP: &str = r#"
(module
  (func (export "evaluate")
    (loop $again
      br $again))
)
"#;

const WASM_MEMORY_EXHAUSTION: &str = r#"
(module
  (import "ic:hooks/before-capability@1" "pass" (func $pass (result i32)))
  (memory (export "memory") 1)
  (func (export "evaluate")
    i32.const 64
    memory.grow
    i32.const -1
    i32.eq
    if
      unreachable
    end
    call $pass
    drop)
)
"#;

/// Observer-shaped sibling of `WASM_MEMORY_EXHAUSTION`. Importing
/// `before-capability::pass` from an observer-point linker fails at
/// install time (correctly), so observer memory-exhaustion tests use this
/// module instead. The body asks for 64 pages (4 MiB) — well beyond the
/// 1 MiB observer budget the test installs — and traps via `unreachable`
/// when wasmtime denies the grow. The trap is what the test asserts gets
/// classified as FailIsolated.
const WASM_OBSERVER_MEMORY_EXHAUSTION: &str = r#"
(module
  (import "ic:hooks/observer@1" "note" (func $note (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "evaluate")
    i32.const 64
    memory.grow
    i32.const -1
    i32.eq
    if
      unreachable
    end
    i32.const 0
    i32.const 0
    call $note
    drop)
)
"#;

const WASM_PROMPT_SINK_OVERFLOW: &str = r#"
(module
  (import "ic:hooks/before-prompt@1" "add_envelope_snippet"
    (func $add_envelope_snippet (param i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "x")
  (func (export "evaluate")
    (local $i i32)
    (loop $again
      i32.const 8
      i32.const 1
      i32.const 0
      call $add_envelope_snippet
      drop
      local.get $i
      i32.const 1
      i32.add
      local.tee $i
      i32.const 65
      i32.lt_s
      br_if $again))
)
"#;

const WASM_PROMPT_HUGE_STRING_LEN: &str = r#"
(module
  (import "ic:hooks/before-prompt@1" "add_envelope_snippet"
    (func $add_envelope_snippet (param i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "evaluate")
    i32.const 0
    i32.const 2147483647
    i32.const 0
    call $add_envelope_snippet
    drop)
)
"#;

const WASM_PROMPT_METADATA_BYTE_OVERFLOW: &str = r#"
(module
  (import "ic:hooks/before-prompt@1" "add_milestone_metadata"
    (func $add_milestone_metadata (param i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx")
  (func (export "evaluate")
    (local $i i32)
    (loop $again
      i32.const 0
      i32.const 8
      i32.const 65
      call $add_milestone_metadata
      drop
      local.get $i
      i32.const 1
      i32.add
      local.tee $i
      i32.const 64
      i32.lt_s
      br_if $again))
)
"#;

const WASM_UNSUPPORTED_IMPORT: &str = r#"
(module
  (import "ic:hooks/before-capability@1" "not_allowed" (func $not_allowed))
  (func (export "evaluate")
    call $not_allowed)
)
"#;

/// Reads the host-supplied context blob via the `ic:hooks/context@1`
/// imports, then denies. The size check serves two purposes: it asserts
/// that the host populates a non-empty blob, and it forces a real
/// `ctx_read` invocation (whose return value is asserted in the read
/// path). If either contract regresses, the module falls through to the
/// `(unreachable)` trap, which the dispatcher classifies as a `Panic`
/// rather than the expected `Decision::Deny` — the test then fails.
/// Critical #1 on PR #3634.
const WASM_CTX_READ_THEN_DENY: &str = r#"
(module
  (import "ic:hooks/context@1" "ctx_size" (func $ctx_size (result i32)))
  (import "ic:hooks/context@1" "ctx_read"
    (func $ctx_read (param i32 i32) (result i32)))
  (import "ic:hooks/before-capability@1" "deny"
    (func $deny (param i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "evaluate")
    (local $size i32)
    (local $read i32)
    call $ctx_size
    local.set $size
    local.get $size
    i32.const 1
    i32.lt_s
    if
      unreachable
    end
    i32.const 0
    local.get $size
    call $ctx_read
    local.set $read
    local.get $read
    local.get $size
    i32.ne
    if
      unreachable
    end
    i32.const 1
    call $deny
    drop)
)
"#;

// ─── Fixture for building hosts with the factory ───────────────────────────

struct Fixture {
    thread_service: Arc<InMemorySessionThreadService>,
    checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    loop_checkpoint_store: Arc<InMemoryTurnStateStore>,
    milestone_sink: Arc<InMemoryLoopHostMilestoneSink>,
    gateway: Arc<UnusedGateway>,
    thread_scope: ThreadScope,
    actor_id: UserId,
    claimed: ClaimedTurnRun,
    context: LoopRunContext,
    surface_version: CapabilitySurfaceVersion,
}

impl Fixture {
    async fn new() -> Self {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store = Arc::new(InMemoryTurnStateStore::default());
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
            actor: Some(TurnActor::new(user_id.clone())),
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
            actor_id: user_id,
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
            Arc::new(StaticTurnStateStore::new(self.claimed.state.clone())),
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

fn router_backed_factory(
    fixture: &Fixture,
    router: Arc<InMemoryHookGateRouter>,
) -> RouterBackedHookGateRefFactory {
    let run_context = fixture.context.clone();
    let actor = HookGateActorBinding::new(fixture.actor_id.clone());
    let router: Arc<dyn HookGateRouter> = router;
    RouterBackedHookGateRefFactory::try_new(router, chrono::Duration::seconds(30), move || {
        HookGateReservationContext::new(run_context.clone(), actor.clone())
    })
    .expect("router-backed hook gate-ref factory accepts positive ttl")
}

fn runtime_scope(fixture: &Fixture) -> ResourceScope {
    ResourceScope {
        tenant_id: fixture.context.scope.tenant_id.clone(),
        user_id: fixture.actor_id.clone(),
        agent_id: fixture.context.scope.agent_id.clone(),
        project_id: fixture.context.scope.project_id.clone(),
        mission_id: None,
        thread_id: Some(fixture.context.thread_id.clone()),
        invocation_id: InvocationId::new(),
    }
}

fn runtime_capability(capability_id: &str) -> CapabilityId {
    CapabilityId::new(capability_id).expect("runtime capability id literal is valid")
}

fn event_log_subscription(
    log: Arc<InMemoryDurableEventLog>,
    stream: EventStreamKey,
    after: RuntimeEventCursor,
) -> EventTriggeredHookSubscription {
    let log: Arc<dyn DurableEventLog> = log;
    EventTriggeredHookSubscription::new(log, stream, ReadScope::any(), after)
        .with_poll_interval(Duration::from_millis(5))
        .with_batch_limit(16)
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

fn expect_denied_with_summary(
    outcome: CapabilityOutcome,
    expected_kind: &str,
    expected_summary: &str,
) {
    match outcome {
        CapabilityOutcome::Denied(denied) => {
            assert_eq!(
                denied.reason_kind,
                CapabilityDeniedReasonKind::unknown(expected_kind)
                    .expect("expected reason kind literal is valid"),
                "denied reason_kind did not match"
            );
            assert_eq!(denied.safe_summary, expected_summary);
        }
        other => panic!("expected CapabilityOutcome::Denied, got {other:?}"),
    }
}

async fn wait_for_seen_events(
    seen: &Arc<Mutex<Vec<SeenRuntimeEvent>>>,
    expected: usize,
) -> Vec<SeenRuntimeEvent> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = seen.lock().expect("seen-event mutex not poisoned").clone();
        if snapshot.len() >= expected {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected} event-triggered hook calls, saw {}",
            snapshot.len()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn event_triggered_hook_matches_runtime_event_kind_subscription() {
    let fixture = Fixture::new().await;
    let scope = runtime_scope(&fixture);
    let stream = EventStreamKey::from_scope(&scope);
    let log = Arc::new(InMemoryDurableEventLog::new());
    let hook_id = HookId::for_builtin("tests::hooks_integration::failed_hook", HookVersion::ONE);

    log.append(RuntimeEvent::hook_decision_emitted(
        scope.clone(),
        runtime_capability("hooks.test"),
        hook_id.to_hex(),
        "pass",
    ))
    .await
    .expect("append non-matching hook event");
    log.append(RuntimeEvent::hook_failed(
        scope,
        runtime_capability("hooks.test"),
        hook_id.to_hex(),
        "panic",
        "fail_isolated",
    ))
    .await
    .expect("append matching hook event");

    let seen = Arc::new(Mutex::new(Vec::new()));
    let inner = Arc::new(RecordingCapabilityPort::new());
    let _host = fixture
        .factory()
        .with_hook_dispatcher(event_triggered_dispatcher(
            RuntimeEventKind::HookFailed,
            Arc::clone(&seen),
        ))
        .with_event_subscription(event_log_subscription(
            Arc::clone(&log),
            stream,
            RuntimeEventCursor::origin(),
        ))
        .build_text_only_host_with_capabilities(fixture.request(), inner)
        .await
        .expect("host builds with event subscription");

    let events = wait_for_seen_events(&seen, 1).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, RuntimeEventKind::HookFailed);
}

#[tokio::test]
async fn event_triggered_subscription_replays_from_resume_cursor() {
    let fixture = Fixture::new().await;
    let scope = runtime_scope(&fixture);
    let stream = EventStreamKey::from_scope(&scope);
    let log = Arc::new(InMemoryDurableEventLog::new());
    let hook_id = HookId::for_builtin("tests::hooks_integration::resume_hook", HookVersion::ONE);
    let prefix = log
        .append(RuntimeEvent::hook_decision_emitted(
            scope.clone(),
            runtime_capability("hooks.resume"),
            hook_id.to_hex(),
            "pass",
        ))
        .await
        .expect("append prefix event");
    for _ in 0..3 {
        log.append(RuntimeEvent::hook_failed(
            scope.clone(),
            runtime_capability("hooks.resume"),
            hook_id.to_hex(),
            "timeout",
            "fail_isolated",
        ))
        .await
        .expect("append replayed event");
    }

    let first_seen = Arc::new(Mutex::new(Vec::new()));
    {
        let inner = Arc::new(RecordingCapabilityPort::new());
        let _host = fixture
            .factory()
            .with_hook_dispatcher(event_triggered_dispatcher(
                RuntimeEventKind::HookFailed,
                Arc::clone(&first_seen),
            ))
            .with_event_subscription(event_log_subscription(
                Arc::clone(&log),
                stream.clone(),
                prefix.cursor,
            ))
            .build_text_only_host_with_capabilities(fixture.request(), inner)
            .await
            .expect("first host builds with event subscription");
        let events = wait_for_seen_events(&first_seen, 3).await;
        assert_eq!(
            events.iter().map(|event| event.cursor).collect::<Vec<_>>(),
            vec![
                RuntimeEventCursor::new(2),
                RuntimeEventCursor::new(3),
                RuntimeEventCursor::new(4),
            ]
        );
    }

    let second_seen = Arc::new(Mutex::new(Vec::new()));
    let inner = Arc::new(RecordingCapabilityPort::new());
    let _host = fixture
        .factory()
        .with_hook_dispatcher(event_triggered_dispatcher(
            RuntimeEventKind::HookFailed,
            Arc::clone(&second_seen),
        ))
        .with_event_subscription(event_log_subscription(
            Arc::clone(&log),
            stream,
            prefix.cursor,
        ))
        .build_text_only_host_with_capabilities(fixture.request(), inner)
        .await
        .expect("restarted host builds with event subscription");
    let events = wait_for_seen_events(&second_seen, 3).await;
    assert_eq!(
        events.iter().map(|event| event.cursor).collect::<Vec<_>>(),
        vec![
            RuntimeEventCursor::new(2),
            RuntimeEventCursor::new(3),
            RuntimeEventCursor::new(4),
        ]
    );
}

#[tokio::test]
async fn event_triggered_hook_respects_own_capabilities_scope_filter() {
    let fixture = Fixture::new().await;
    let scope = runtime_scope(&fixture);
    let stream = EventStreamKey::from_scope(&scope);
    let log = Arc::new(InMemoryDurableEventLog::new());
    let ext_a = ironclaw_host_api::ExtensionId::new("ext-a").expect("valid ext");
    let ext_b = ironclaw_host_api::ExtensionId::new("ext-b").expect("valid ext");

    log.append(RuntimeEvent::dispatch_succeeded(
        scope.clone(),
        runtime_capability("ext-a.call"),
        ext_a.clone(),
        RuntimeKind::Wasm,
        16,
    ))
    .await
    .expect("append own-provider event");
    log.append(RuntimeEvent::dispatch_succeeded(
        scope,
        runtime_capability("ext-b.call"),
        ext_b,
        RuntimeKind::Wasm,
        16,
    ))
    .await
    .expect("append foreign-provider event");

    let seen = Arc::new(Mutex::new(Vec::new()));
    let inner = Arc::new(RecordingCapabilityPort::new());
    let _host = fixture
        .factory()
        .with_hook_dispatcher(event_triggered_dispatcher_with_scope(
            RuntimeEventKind::DispatchSucceeded,
            Arc::clone(&seen),
            HookBindingScope::OwnCapabilities,
            ext_a.as_str(),
            None,
        ))
        .with_event_subscription(event_log_subscription(
            Arc::clone(&log),
            stream,
            RuntimeEventCursor::origin(),
        ))
        .build_text_only_host_with_capabilities(fixture.request(), inner)
        .await
        .expect("host builds with scoped event hook");

    let events = wait_for_seen_events(&seen, 1).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, RuntimeEventKind::DispatchSucceeded);
    assert_eq!(events[0].provider.as_ref(), Some(&ext_a));
}

#[tokio::test]
async fn event_triggered_own_capabilities_scope_resolves_hook_failed_owner_from_hook_id() {
    let fixture = Fixture::new().await;
    let scope = runtime_scope(&fixture);
    let stream = EventStreamKey::from_scope(&scope);
    let log = Arc::new(InMemoryDurableEventLog::new());
    let ext_a = ironclaw_host_api::ExtensionId::new("ext-a").expect("valid ext");
    let ext_b = ironclaw_host_api::ExtensionId::new("ext-b").expect("valid ext");
    let ext_a_source_hook_id = HookId::derive(
        &ExtensionId(ext_a.as_str().to_string()),
        "0.0.1",
        &HookLocalId("hook-failed-source-a".to_string()),
        HookVersion::ONE,
    );
    let ext_b_source_hook_id = HookId::derive(
        &ExtensionId(ext_b.as_str().to_string()),
        "0.0.1",
        &HookLocalId("hook-failed-source-b".to_string()),
        HookVersion::ONE,
    );
    let ext_a_source_hook_hex = ext_a_source_hook_id.to_hex();
    let ext_b_source_hook_hex = ext_b_source_hook_id.to_hex();

    log.append(RuntimeEvent::hook_failed(
        scope.clone(),
        runtime_capability("hooks.ext_a.first"),
        ext_a_source_hook_hex.clone(),
        "panic",
        "fail_isolated",
    ))
    .await
    .expect("append ext-A hook failure event");
    log.append(RuntimeEvent::hook_failed(
        scope.clone(),
        runtime_capability("hooks.ext_b"),
        ext_b_source_hook_hex.clone(),
        "panic",
        "fail_isolated",
    ))
    .await
    .expect("append ext-B hook failure event");
    log.append(RuntimeEvent::hook_failed(
        scope,
        runtime_capability("hooks.ext_a.second"),
        ext_a_source_hook_hex.clone(),
        "panic",
        "fail_isolated",
    ))
    .await
    .expect("append second ext-A hook failure event");

    let seen = Arc::new(Mutex::new(Vec::new()));
    let inert_seen = Arc::new(Mutex::new(Vec::new()));
    let subscription_hook_id = HookId::derive(
        &ExtensionId(ext_a.as_str().to_string()),
        "0.0.1",
        &HookLocalId("hook-failed-own-capabilities-subscription".to_string()),
        HookVersion::ONE,
    );
    let dispatcher = HookDispatcherBuilder::new(HookRegistry::new())
        .with_timeout(Duration::from_millis(500))
        .install_installed_event_triggered(
            subscription_hook_id,
            HookPhase::Telemetry,
            RuntimeEventKind::HookFailed,
            ext_a.clone(),
            HookBindingScope::OwnCapabilities,
            Box::new(RecordingEventTriggeredHook {
                seen: Arc::clone(&seen),
                delay: None,
            }),
        )
        .expect("install ext-A HookFailed subscription")
        .install_installed_event_triggered(
            ext_a_source_hook_id,
            HookPhase::Telemetry,
            RuntimeEventKind::DispatchSucceeded,
            ext_a,
            HookBindingScope::Global,
            Box::new(RecordingEventTriggeredHook {
                seen: Arc::clone(&inert_seen),
                delay: None,
            }),
        )
        .expect("install ext-A source hook binding")
        .install_installed_event_triggered(
            ext_b_source_hook_id,
            HookPhase::Telemetry,
            RuntimeEventKind::DispatchSucceeded,
            ext_b,
            HookBindingScope::Global,
            Box::new(RecordingEventTriggeredHook {
                seen: Arc::clone(&inert_seen),
                delay: None,
            }),
        )
        .expect("install ext-B source hook binding")
        .build_arc();
    let inner = Arc::new(RecordingCapabilityPort::new());
    let _host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .with_event_subscription(event_log_subscription(
            Arc::clone(&log),
            stream,
            RuntimeEventCursor::origin(),
        ))
        .build_text_only_host_with_capabilities(fixture.request(), inner)
        .await
        .expect("host builds with scoped hook-failure event subscription");

    let events = wait_for_seen_events(&seen, 2).await;
    assert_eq!(events.len(), 2);
    assert!(
        events.iter().all(|event| event.provider.is_none()),
        "HookFailed constructors leave provider unset; test must exercise hook_id owner lookup"
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.hook_id.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some(ext_a_source_hook_hex.as_str()),
            Some(ext_a_source_hook_hex.as_str())
        ],
        "ext-A OwnCapabilities subscription must fire only for ext-A hook failures"
    );
}

#[tokio::test]
async fn event_triggered_sink_is_observer_only_at_caller_boundary() {
    let fixture = Fixture::new().await;
    let scope = runtime_scope(&fixture);
    let hook_id = HookId::for_builtin(
        "tests::hooks_integration::observer_only_event_hook",
        HookVersion::ONE,
    );
    let event = RuntimeEvent::hook_failed(
        scope,
        runtime_capability("hooks.observer_only"),
        hook_id.to_hex(),
        "panic",
        "fail_isolated",
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let dispatcher = event_triggered_dispatcher(RuntimeEventKind::HookFailed, Arc::clone(&seen));

    let outcome = dispatcher
        .dispatch_event_triggered_at(
            fixture.context.scope.tenant_id.clone(),
            RuntimeEventCursor::new(1),
            &event,
        )
        .await;

    assert!(outcome.failures.is_empty());
    assert_eq!(outcome.facts.len(), 1);
    assert_eq!(seen.lock().expect("seen-event mutex not poisoned").len(), 1);
}

#[tokio::test]
async fn event_triggered_slow_hook_does_not_block_event_emit_caller() {
    let fixture = Fixture::new().await;
    let scope = runtime_scope(&fixture);
    let stream = EventStreamKey::from_scope(&scope);
    let log = Arc::new(InMemoryDurableEventLog::new());
    let sink = DurableEventSink::new(Arc::clone(&log) as Arc<dyn DurableEventLog>);
    let hook_id = HookId::for_builtin(
        "tests::hooks_integration::slow_event_hook",
        HookVersion::ONE,
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let inner = Arc::new(RecordingCapabilityPort::new());
    let _host = fixture
        .factory()
        .with_hook_dispatcher(event_triggered_dispatcher_with_scope(
            RuntimeEventKind::HookFailed,
            Arc::clone(&seen),
            HookBindingScope::Global,
            "integration-tests",
            Some(Duration::from_millis(100)),
        ))
        .with_event_subscription(event_log_subscription(
            Arc::clone(&log),
            stream,
            RuntimeEventCursor::origin(),
        ))
        .build_text_only_host_with_capabilities(fixture.request(), inner)
        .await
        .expect("host builds with event subscription");

    let started = Instant::now();
    for _ in 0..10 {
        sink.emit(RuntimeEvent::hook_failed(
            scope.clone(),
            runtime_capability("hooks.slow"),
            hook_id.to_hex(),
            "timeout",
            "fail_isolated",
        ))
        .await
        .expect("emit event");
    }
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "event emission should not wait for ten 100ms hook callbacks"
    );

    let events = wait_for_seen_events(&seen, 10).await;
    assert_eq!(events.len(), 10);
    assert!(
        events
            .iter()
            .all(|event| event.kind == RuntimeEventKind::HookFailed)
    );
}

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
async fn wasm_before_capability_hook_denies_through_factory() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let (dispatcher, _hook_id) = wasm_dispatcher_from_wat(
        "wasm-deny",
        HookManifestKind::BeforeCapability,
        WASM_DENY_HOOK,
        WasmBudget::default(),
    );

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with wasm hook dispatcher installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability returns denied outcome");

    expect_denied_with_summary(outcome, "hook_denied", "hook_predicate_denied");
    assert!(
        inner.invocations().is_empty(),
        "inner port must NOT be invoked when wasm hook denies"
    );
}

#[tokio::test]
async fn wasm_before_capability_hook_reads_context_blob() {
    // Critical #1 on PR #3634: the dispatcher MUST plumb the hook context
    // through to the guest. This module asks `ctx_size` for the context
    // blob's byte length, reads it via `ctx_read`, and only denies if both
    // host imports report a non-empty, fully-readable payload. A regression
    // (e.g., empty blob, wrong return value) traps before reaching `deny`,
    // which the dispatcher classifies as a panic — failing this assertion.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let (dispatcher, _hook_id) = wasm_dispatcher_from_wat(
        "wasm-ctx-read",
        HookManifestKind::BeforeCapability,
        WASM_CTX_READ_THEN_DENY,
        WasmBudget::default(),
    );

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with wasm hook dispatcher installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability returns denied outcome");

    expect_denied_with_summary(outcome, "hook_denied", "hook_predicate_denied");
    assert!(
        inner.invocations().is_empty(),
        "inner port must NOT be invoked when wasm hook denies after reading context"
    );
}

#[tokio::test]
async fn wasm_fuel_exhaustion_fails_closed_for_gate() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let (dispatcher, _hook_id) = wasm_dispatcher_from_wat(
        "wasm-loop-gate",
        HookManifestKind::BeforeCapability,
        WASM_INFINITE_LOOP,
        WasmBudget {
            fuel: 1_000,
            memory_mb: 4,
            wall_ms: 50,
        },
    );

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with looping wasm gate hook installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability returns denied outcome");

    expect_denied_with(outcome, "hook_denied");
    assert!(
        inner.invocations().is_empty(),
        "fuel-exhausted gate hook must fail closed before inner port"
    );
}

#[tokio::test]
async fn wasm_fuel_exhaustion_fails_isolated_for_observer() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let (dispatcher, _hook_id) = wasm_dispatcher_from_wat(
        "wasm-loop-observer",
        HookManifestKind::AfterCapability,
        WASM_INFINITE_LOOP,
        WasmBudget {
            fuel: 1_000,
            memory_mb: 4,
            wall_ms: 50,
        },
    );

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with looping wasm observer installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.allowed"))
        .await
        .expect("invoke_capability returns completed outcome");

    assert!(
        matches!(outcome, CapabilityOutcome::Completed(_)),
        "observer failure must be isolated; got {outcome:?}"
    );
    assert_eq!(
        inner.invocations().len(),
        1,
        "observer failure must not block the inner capability call"
    );
}

#[tokio::test]
async fn wasm_memory_exhaustion_fails_closed_for_gate() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let (dispatcher, _hook_id) = wasm_dispatcher_from_wat(
        "wasm-memory",
        HookManifestKind::BeforeCapability,
        WASM_MEMORY_EXHAUSTION,
        WasmBudget {
            fuel: 100_000,
            memory_mb: 1,
            wall_ms: 50,
        },
    );

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with memory-exhausting wasm gate hook installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability returns denied outcome");

    expect_denied_with(outcome, "hook_denied");
    assert!(
        inner.invocations().is_empty(),
        "memory-exhausted gate hook must fail closed before inner port"
    );
}

#[tokio::test]
async fn wasm_wall_clock_timeout_fires_for_gate() {
    // Test #11 on PR #3634: the dispatcher's outer `tokio::time::timeout`
    // arm was previously unreachable because the synchronous wasmtime call
    // ran on the tokio executor itself, blocking the timer that would
    // otherwise fire it. After moving to `spawn_blocking`, that arm IS
    // reachable — this test exercises it by giving the WASM module a wide
    // wasmtime budget (so wasmtime's own epoch-interrupt + fuel cap do NOT
    // fire first) and the dispatcher a tight timeout.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let (dispatcher, _hook_id) = wasm_dispatcher_from_wat_with_timeout(
        "wasm-wallclock-gate",
        HookManifestKind::BeforeCapability,
        WASM_INFINITE_LOOP,
        WasmBudget {
            fuel: 1_000_000_000,
            memory_mb: 4,
            wall_ms: 5_000,
        },
        Some(Duration::from_millis(20)),
    );

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with wall-clock-bound wasm gate hook installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.blocked"))
        .await
        .expect("invoke_capability returns denied outcome");

    expect_denied_with(outcome, "hook_denied");
    assert!(
        inner.invocations().is_empty(),
        "wall-clock-timeout gate hook must fail closed before inner port"
    );
}

#[tokio::test]
async fn wasm_wall_clock_timeout_fires_for_observer() {
    // Test #12 on PR #3634: parallels the gate wall-clock test for the
    // observer dispatch path. Observer failures are FailIsolated so the
    // outer capability still completes.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let (dispatcher, _hook_id) = wasm_dispatcher_from_wat_with_timeout(
        "wasm-wallclock-observer",
        HookManifestKind::AfterCapability,
        WASM_INFINITE_LOOP,
        WasmBudget {
            fuel: 1_000_000_000,
            memory_mb: 4,
            wall_ms: 5_000,
        },
        Some(Duration::from_millis(20)),
    );

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with wall-clock-bound wasm observer installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.allowed"))
        .await
        .expect("invoke_capability returns completed outcome");

    assert!(
        matches!(outcome, CapabilityOutcome::Completed(_)),
        "observer wall-clock timeout must be isolated; got {outcome:?}"
    );
    assert_eq!(
        inner.invocations().len(),
        1,
        "observer wall-clock failure must not block the inner capability call"
    );
}

#[tokio::test]
async fn wasm_memory_exhaustion_fails_isolated_for_observer() {
    // Test #13 on PR #3634: parallels `wasm_memory_exhaustion_fails_closed_for_gate`
    // for the observer dispatch path. Observers run after the capability
    // completes, so the failure-policy matrix turns a trap from a wasm
    // observer into FailIsolated (the outer capability still succeeds) —
    // exactly the symmetry the threat model documents but that the existing
    // test set didn't exercise.
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let (dispatcher, _hook_id) = wasm_dispatcher_from_wat(
        "wasm-memory-observer",
        HookManifestKind::AfterCapability,
        WASM_OBSERVER_MEMORY_EXHAUSTION,
        WasmBudget {
            fuel: 100_000,
            memory_mb: 1,
            wall_ms: 50,
        },
    );

    let host = fixture
        .factory()
        .with_hook_dispatcher(dispatcher)
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with memory-exhausting wasm observer installed");

    let outcome = host
        .invoke_capability(invocation(&surface_version, "cap.allowed"))
        .await
        .expect("invoke_capability returns completed outcome");

    assert!(
        matches!(outcome, CapabilityOutcome::Completed(_)),
        "observer memory exhaustion must be isolated; got {outcome:?}"
    );
    assert_eq!(
        inner.invocations().len(),
        1,
        "observer memory failure must not block the inner capability call"
    );
}

// FIXME(hooks-wasm): Installed-tier `before_prompt` WASM hooks cannot be
// installed via the public manifest path today: the registry C3 fix
// (finding #2 on PR #3573) rejects `OwnCapabilities` at `BeforePrompt`
// (no per-capability invocation context), while the manifest validator
// rejects `SameTenant + before_prompt` outright. The original branch tip
// (`origin/hooks-fu-wasm-runtime` @ 571efdf67) had the same gap — these
// tests were authored against the WASM scaffolding before the registry
// check was tightened, and never updated. They exercise the WASM sink-call
// / huge-string-len / metadata-byte budget paths, which are point-agnostic;
// porting the helper to install via `BeforeCapability` (or adding a
// `Global` manifest scope) is the natural follow-up. Tracked under the
// "deferred items" list in the new PR description.
#[tokio::test]
#[ignore = "BeforePrompt WASM install path needs manifest/registry alignment (carry-forward from #3634)"]
async fn wasm_sink_call_budget_overflow_is_malformed_and_fails_closed() {
    let dispatcher = wasm_before_prompt_dispatcher_from_wat(
        "wasm-sink-overflow",
        WASM_PROMPT_SINK_OVERFLOW,
        WasmBudget::default(),
    );
    let ctx = BeforePromptHookContext::new(
        TenantId::new("tenant-hooks-integration").expect("valid tenant"),
        4 * 1024,
    );

    let outcome = dispatcher.dispatch_before_prompt(&ctx).await;

    assert!(
        outcome.patches.is_empty(),
        "malformed prompt hook output must not be applied"
    );
    assert_eq!(outcome.failures.len(), 1, "expected one hook failure");
    let failure = &outcome.failures[0];
    assert_eq!(failure.category, FailureCategory::Malformed);
    assert_eq!(failure.disposition, FailureDisposition::FailClosed);
    assert_eq!(
        failure.reason.as_str(),
        "wasm hook exceeded sink-call budget"
    );
}

#[tokio::test]
#[ignore = "BeforePrompt WASM install path needs manifest/registry alignment (carry-forward from #3634)"]
async fn wasm_huge_guest_string_len_is_malformed_without_host_allocation() {
    let dispatcher = wasm_before_prompt_dispatcher_from_wat(
        "wasm-huge-string-len",
        WASM_PROMPT_HUGE_STRING_LEN,
        WasmBudget::default(),
    );
    let ctx = BeforePromptHookContext::new(
        TenantId::new("tenant-hooks-integration").expect("valid tenant"),
        4 * 1024,
    );

    let outcome = dispatcher.dispatch_before_prompt(&ctx).await;

    assert!(
        outcome.patches.is_empty(),
        "malformed prompt hook output must not be applied"
    );
    assert_eq!(outcome.failures.len(), 1, "expected one hook failure");
    let failure = &outcome.failures[0];
    assert_eq!(failure.category, FailureCategory::Malformed);
    assert_eq!(failure.disposition, FailureDisposition::FailClosed);
    assert_eq!(
        failure.reason.as_str(),
        "wasm hook supplied an invalid string pointer"
    );
}

#[tokio::test]
#[ignore = "BeforePrompt WASM install path needs manifest/registry alignment (carry-forward from #3634)"]
async fn wasm_metadata_byte_budget_overflow_is_malformed_and_fails_closed() {
    let dispatcher = wasm_before_prompt_dispatcher_from_wat(
        "wasm-metadata-byte-overflow",
        WASM_PROMPT_METADATA_BYTE_OVERFLOW,
        WasmBudget::default(),
    );
    let ctx = BeforePromptHookContext::new(
        TenantId::new("tenant-hooks-integration").expect("valid tenant"),
        4 * 1024,
    );

    let outcome = dispatcher.dispatch_before_prompt(&ctx).await;

    assert!(
        outcome.patches.is_empty(),
        "malformed prompt hook output must not be applied"
    );
    assert_eq!(outcome.failures.len(), 1, "expected one hook failure");
    let failure = &outcome.failures[0];
    assert_eq!(failure.category, FailureCategory::Malformed);
    assert_eq!(failure.disposition, FailureDisposition::FailClosed);
    assert_eq!(
        failure.reason.as_str(),
        "wasm hook exceeded total prompt-patch byte budget"
    );
}

#[test]
fn wasm_module_substitution_is_rejected_by_checkpoint_replay_guard() {
    let (_dispatcher_a, hook_id_a) = wasm_dispatcher_from_wat(
        "wasm-substitute",
        HookManifestKind::BeforeCapability,
        WASM_DENY_HOOK,
        WasmBudget::default(),
    );
    let (dispatcher_b, hook_id_b) = wasm_dispatcher_from_wat(
        "wasm-substitute",
        HookManifestKind::BeforeCapability,
        WASM_MEMORY_EXHAUSTION,
        WasmBudget::default(),
    );

    assert_ne!(
        hook_id_a, hook_id_b,
        "same local id with different module bytes must derive a different hook id"
    );
    let err = dispatcher_b
        .validate_checkpoint_hook_ids_for_replay(&[hook_id_a])
        .expect_err("checkpoint pinned to module A must not replay against module B registry");
    match err {
        HookError::UnknownHook(id) => assert_eq!(id, hook_id_a),
        other => panic!("expected UnknownHook replay refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn wasm_unsupported_host_import_is_rejected_at_install_time() {
    // serrrfirat finding #3: bad-import modules must be rejected at
    // registration so they never reach live dispatch. Prior behavior
    // accepted the install and only failed at first invocation.
    let resolver = Arc::new(InMemoryWasmHookModules::with_wat(
        "wasm-bad-import",
        WASM_UNSUPPORTED_IMPORT,
    ));
    let runtime = Arc::new(WasmHookRuntime::new(resolver).expect("wasm runtime"));
    let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()))
        .with_wasm_runtime(runtime)
        .with_verified_grants(["integration-test-wasm-hooks".to_string()]);
    let entry = HookManifestEntry::new(
        HookLocalId::new("wasm-bad-import").expect("valid HookLocalId in test"),
        HookManifestKind::BeforeCapability,
        HookManifestBody::Wasm {
            export: "evaluate".to_string(),
            budget: WasmBudget::default(),
        },
    )
    .with_scope(HookManifestScope::SameTenant)
    .with_requires_grant("integration-test-wasm-hooks");
    let builder = HookDispatcherBuilder::new(HookRegistry::new());
    let result = registrar.install(
        ironclaw_host_api::ExtensionId::new("integration-tests").expect("valid ext id"),
        "0.0.1".to_string(),
        vec![entry],
        builder,
    );
    match result {
        Err(HookError::RegistryConstruction(msg)) => {
            assert!(
                msg.contains("not_allowed") || msg.contains("imports"),
                "expected install-time rejection citing the unsupported import; got: {msg}"
            );
        }
        Err(other) => panic!("expected RegistryConstruction error, got {other:?}"),
        Ok(_) => panic!("install must reject bad-import wasm hook before live dispatch"),
    }
}

#[tokio::test]
async fn wasm_missing_export_is_rejected_at_install_time() {
    // Same surface as the bad-import test: a module that compiles cleanly
    // but lacks the manifest-declared export must fail at install rather
    // than survive to first dispatch.
    const WASM_MISSING_EXPORT: &str = r#"
(module
  (import "ic:hooks/before-capability@1" "deny" (func $deny (param i32) (result i32)))
  (func (export "other_name")
    i32.const 1
    call $deny
    drop)
)
"#;
    let resolver = Arc::new(InMemoryWasmHookModules::with_wat(
        "wasm-missing-export",
        WASM_MISSING_EXPORT,
    ));
    let runtime = Arc::new(WasmHookRuntime::new(resolver).expect("wasm runtime"));
    let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()))
        .with_wasm_runtime(runtime)
        .with_verified_grants(["integration-test-wasm-hooks".to_string()]);
    let entry = HookManifestEntry::new(
        HookLocalId::new("wasm-missing-export").expect("valid HookLocalId in test"),
        HookManifestKind::BeforeCapability,
        HookManifestBody::Wasm {
            export: "evaluate".to_string(),
            budget: WasmBudget::default(),
        },
    )
    .with_scope(HookManifestScope::SameTenant)
    .with_requires_grant("integration-test-wasm-hooks");
    let builder = HookDispatcherBuilder::new(HookRegistry::new());
    let result = registrar.install(
        ironclaw_host_api::ExtensionId::new("integration-tests").expect("valid ext id"),
        "0.0.1".to_string(),
        vec![entry],
        builder,
    );
    match result {
        Err(HookError::RegistryConstruction(msg)) => {
            assert!(
                msg.contains("evaluate") || msg.contains("export"),
                "expected install-time rejection citing the missing export; got: {msg}"
            );
        }
        Err(other) => panic!("expected RegistryConstruction error, got {other:?}"),
        Ok(_) => panic!("install must reject missing-export wasm hook before live dispatch"),
    }
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
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let router = Arc::new(InMemoryHookGateRouter::new());
    let factory = router_backed_factory(&fixture, Arc::clone(&router));
    let request = invocation(&surface_version, "cap.blocked");

    let host = fixture
        .factory()
        .with_hook_dispatcher(pause_approval_dispatcher())
        .with_hook_gate_ref_factory_builder({
            let f: Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory> = Arc::new(factory);
            move |_| Arc::clone(&f)
        })
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher installed");

    let outcome = host
        .invoke_capability(request.clone())
        .await
        .expect("invoke_capability returns a (suspended) outcome, not an error");

    let gate_ref = match outcome {
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
            gate_ref
        }
        other => panic!("expected ApprovalRequired, got {other:?}"),
    };
    assert!(
        inner.invocations().is_empty(),
        "inner port must NOT be invoked when a hook pauses; got {:?}",
        inner.invocations()
    );

    let resolution_request = HookGateResolutionRequest::for_invocation(
        gate_ref.clone(),
        HookGateActorBinding::new(fixture.actor_id.clone()),
        fixture.context.clone(),
        &request,
    )
    .expect("resolution request can be derived from invocation");
    let resolution = router
        .resolve(resolution_request.clone())
        .await
        .expect("gateway consumes issued hook approval ref");
    assert_eq!(resolution.gate_ref, gate_ref);
    assert_eq!(resolution.capability_id.as_str(), "cap.blocked");
    assert_eq!(
        resolution.arguments_digest,
        hook_gate_arguments_digest(&request),
        "gateway reservation must carry the exact gated invocation digest"
    );

    let replay = router
        .resolve(resolution_request)
        .await
        .expect_err("gate refs are one-shot");
    assert!(
        replay.is_already_consumed(),
        "replay must be rejected as consumed, got {replay:?}"
    );
}

#[tokio::test]
async fn router_backed_pause_approval_gate_ref_rejects_cross_actor_resolution() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let router = Arc::new(InMemoryHookGateRouter::new());
    let factory = router_backed_factory(&fixture, Arc::clone(&router));
    let request = invocation(&surface_version, "cap.blocked");

    let host = fixture
        .factory()
        .with_hook_dispatcher(pause_approval_dispatcher())
        .with_hook_gate_ref_factory_builder({
            let f: Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory> = Arc::new(factory);
            move |_| Arc::clone(&f)
        })
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher installed");

    let outcome = host
        .invoke_capability(request.clone())
        .await
        .expect("invoke_capability returns a (suspended) outcome, not an error");

    let CapabilityOutcome::ApprovalRequired { gate_ref, .. } = outcome else {
        panic!("expected ApprovalRequired, got {outcome:?}");
    };

    let wrong_actor = UserId::new("other-hooks-user").expect("user id literal is valid");
    let cross_actor = HookGateResolutionRequest::for_invocation(
        gate_ref.clone(),
        HookGateActorBinding::new(wrong_actor),
        fixture.context.clone(),
        &request,
    )
    .expect("resolution request can be derived from invocation");
    let err = router
        .resolve(cross_actor)
        .await
        .expect_err("wrong actor must not consume another actor's gate ref");
    assert!(
        err.is_actor_mismatch(),
        "wrong actor must be rejected before consumption, got {err:?}"
    );

    let right_actor = HookGateResolutionRequest::for_invocation(
        gate_ref,
        HookGateActorBinding::new(fixture.actor_id.clone()),
        fixture.context.clone(),
        &request,
    )
    .expect("resolution request can be derived from invocation");
    router
        .resolve(right_actor)
        .await
        .expect("cross-actor rejection must not consume the ref");
}

#[tokio::test]
async fn router_backed_gate_ref_factory_sources_context_per_mint_when_reused() {
    let fixture = Fixture::new().await;
    let router = Arc::new(InMemoryHookGateRouter::new());
    let surface_version = fixture.surface_version.clone();

    let actor_a = HookGateActorBinding::new(
        UserId::new("ext-a-hooks-user").expect("actor A id literal is valid"),
    );
    let actor_b = HookGateActorBinding::new(
        UserId::new("ext-b-hooks-user").expect("actor B id literal is valid"),
    );
    let live_context = Arc::new(Mutex::new(HookGateReservationContext::new(
        fixture.context.clone(),
        actor_a.clone(),
    )));
    let context_source = Arc::clone(&live_context);
    let router_for_factory: Arc<dyn HookGateRouter> = router.clone();
    let gate_ref_factory = RouterBackedHookGateRefFactory::try_new(
        router_for_factory,
        chrono::Duration::seconds(30),
        move || {
            context_source
                .lock()
                .expect("live hook gate context mutex not poisoned")
                .clone()
        },
    )
    .expect("router-backed hook gate-ref factory accepts positive ttl");
    let host_factory = fixture
        .factory()
        .with_hook_dispatcher(pause_approval_dispatcher())
        .with_hook_gate_ref_factory_builder({
            let f: Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory> =
                Arc::new(gate_ref_factory);
            move |_| Arc::clone(&f)
        });

    let inner_a = Arc::new(RecordingCapabilityPort::new());
    let request_a = fixture.request();
    let context_a = request_a.loop_run_context.clone();
    let host_a = host_factory
        .build_text_only_host_with_capabilities(request_a, inner_a.clone())
        .await
        .expect("first host builds");
    let invocation_a = invocation(&surface_version, "cap.blocked");
    let outcome_a = host_a
        .invoke_capability(invocation_a.clone())
        .await
        .expect("first invoke returns outcome");
    let CapabilityOutcome::ApprovalRequired {
        gate_ref: gate_ref_a,
        ..
    } = outcome_a
    else {
        panic!("expected first build ApprovalRequired, got {outcome_a:?}");
    };
    router
        .resolve(
            HookGateResolutionRequest::for_invocation(
                gate_ref_a,
                actor_a.clone(),
                context_a.clone(),
                &invocation_a,
            )
            .expect("first resolution request can be derived from invocation"),
        )
        .await
        .expect("first build gate resolves with actor A/context A");

    let mut request_b = fixture.request();
    let turn_id_b = ironclaw_turns::TurnId::new();
    let run_id_b = TurnRunId::new();
    request_b.loop_run_context = LoopRunContext::new(
        request_b.loop_run_context.scope.clone(),
        turn_id_b,
        run_id_b,
        request_b.loop_run_context.resolved_run_profile.clone(),
    );
    request_b.claimed_run.state.turn_id = turn_id_b;
    request_b.claimed_run.state.run_id = run_id_b;
    let context_b = request_b.loop_run_context.clone();
    *live_context
        .lock()
        .expect("live hook gate context mutex not poisoned") =
        HookGateReservationContext::new(context_b.clone(), actor_b.clone());

    let inner_b = Arc::new(RecordingCapabilityPort::new());
    let host_b = host_factory
        .build_text_only_host_with_capabilities(request_b, inner_b.clone())
        .await
        .expect("second host builds");
    let invocation_b = invocation(&surface_version, "cap.blocked");
    let outcome_b = host_b
        .invoke_capability(invocation_b.clone())
        .await
        .expect("second invoke returns outcome");
    let CapabilityOutcome::ApprovalRequired {
        gate_ref: gate_ref_b,
        ..
    } = outcome_b
    else {
        panic!("expected second build ApprovalRequired, got {outcome_b:?}");
    };

    router
        .resolve(
            HookGateResolutionRequest::for_invocation(
                gate_ref_b.clone(),
                actor_a,
                context_a,
                &invocation_b,
            )
            .expect("stale resolution request can be derived from invocation"),
        )
        .await
        .expect_err("second build gate must not resolve with actor/context from build A");
    router
        .resolve(
            HookGateResolutionRequest::for_invocation(
                gate_ref_b,
                actor_b,
                context_b,
                &invocation_b,
            )
            .expect("second resolution request can be derived from invocation"),
        )
        .await
        .expect("second build gate resolves with actor B/context B");
}

#[tokio::test]
async fn router_backed_pause_approval_gate_ref_expires_after_ttl() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let router = Arc::new(InMemoryHookGateRouter::new());
    let router_for_factory: Arc<dyn HookGateRouter> = router.clone();
    let run_context = fixture.context.clone();
    let actor = HookGateActorBinding::new(fixture.actor_id.clone());
    let factory = RouterBackedHookGateRefFactory::try_new(
        router_for_factory,
        chrono::Duration::milliseconds(1),
        move || HookGateReservationContext::new(run_context.clone(), actor.clone()),
    )
    .expect("positive ttl is accepted");
    let request = invocation(&surface_version, "cap.blocked");

    let host = fixture
        .factory()
        .with_hook_dispatcher(pause_approval_dispatcher())
        .with_hook_gate_ref_factory_builder({
            let f: Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory> = Arc::new(factory);
            move |_| Arc::clone(&f)
        })
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher installed");

    let outcome = host
        .invoke_capability(request.clone())
        .await
        .expect("invoke_capability returns a (suspended) outcome, not an error");

    match outcome {
        CapabilityOutcome::ApprovalRequired { gate_ref, .. } => {
            tokio::time::sleep(Duration::from_millis(5)).await;
            let err = router
                .resolve(
                    HookGateResolutionRequest::for_invocation(
                        gate_ref,
                        HookGateActorBinding::new(fixture.actor_id.clone()),
                        fixture.context.clone(),
                        &request,
                    )
                    .expect("resolution request can be derived from invocation"),
                )
                .await
                .expect_err("expired gate ref must be rejected");
            assert!(err.is_expired(), "expected expired error, got {err:?}");
        }
        other => panic!("expected ApprovalRequired, got {other:?}"),
    }
    assert!(
        inner.invocations().is_empty(),
        "inner port must NOT be invoked when a hook pauses; got {:?}",
        inner.invocations()
    );
}

#[tokio::test]
async fn router_backed_pause_approval_gate_ref_rejects_backdated_resolution_after_ttl() {
    let fixture = Fixture::new().await;
    let inner = Arc::new(RecordingCapabilityPort::new());
    let surface_version = fixture.surface_version.clone();
    let router = Arc::new(InMemoryHookGateRouter::new());
    let router_for_factory: Arc<dyn HookGateRouter> = router.clone();
    let run_context = fixture.context.clone();
    let actor = HookGateActorBinding::new(fixture.actor_id.clone());
    let factory = RouterBackedHookGateRefFactory::try_new(
        router_for_factory,
        chrono::Duration::milliseconds(1),
        move || HookGateReservationContext::new(run_context.clone(), actor.clone()),
    )
    .expect("positive ttl is accepted");
    let request = invocation(&surface_version, "cap.blocked");

    let host = fixture
        .factory()
        .with_hook_dispatcher(pause_approval_dispatcher())
        .with_hook_gate_ref_factory_builder({
            let f: Arc<dyn ironclaw_hooks::middleware::HookGateRefFactory> = Arc::new(factory);
            move |_| Arc::clone(&f)
        })
        .build_text_only_host_with_capabilities(fixture.request(), inner.clone())
        .await
        .expect("host builds with hook dispatcher installed");

    let outcome = host
        .invoke_capability(request.clone())
        .await
        .expect("invoke_capability returns a (suspended) outcome, not an error");

    let CapabilityOutcome::ApprovalRequired { gate_ref, .. } = outcome else {
        panic!("expected ApprovalRequired, got {outcome:?}");
    };
    // serrrfirat HIGH regression: `HookGateResolutionRequest` no longer
    // exposes a caller-controllable `resolved_at`. Time authority lives on
    // the router's own wall clock — see `InMemoryHookGateRouter::resolve_gate`
    // where `Utc::now()` is read directly. The TTL was 1ms above and we
    // slept 5ms past reservation, so the router will compute "now is past
    // expires_at" using its own clock; no caller value can override it.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let resolution_request = HookGateResolutionRequest::for_invocation(
        gate_ref,
        HookGateActorBinding::new(fixture.actor_id.clone()),
        fixture.context.clone(),
        &request,
    )
    .expect("resolution request can be derived from invocation");

    let err = router
        .resolve(resolution_request)
        .await
        .expect_err("router-owned clock must enforce TTL regardless of caller intent");
    assert!(err.is_expired(), "expected expired error, got {err:?}");
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
            capability_view: None,
        })
        .await
        .expect("build_prompt_bundle succeeds before stream_model");
    host.stream_model(LoopModelRequest {
        messages: bundle.messages.clone(),
        surface_version: None,
        model_preference: None,
        capability_view: None,
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
        &ExtensionId::new("integration-tests").expect("valid ExtensionId in test"),
        "0.0.1",
        &HookLocalId::new("numeric-sum-amount").expect("valid HookLocalId in test"),
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
        &ExtensionId::new("ext-a").expect("valid ExtensionId in test"),
        "0.0.1",
        &HookLocalId::new("c3-own-scope-deny").expect("valid HookLocalId in test"),
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
        &ExtensionId::new(owning_ext).expect("valid ExtensionId in test"),
        "0.0.1",
        &HookLocalId::new(local_id).expect("valid HookLocalId in test"),
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
            &ExtensionId::new("ext-tele").expect("valid ExtensionId in test"),
            "0.0.1",
            &HookLocalId::new("deny-everything").expect("valid HookLocalId in test"),
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
        &ExtensionId::new("ext-prompt").expect("valid ExtensionId in test"),
        "0.0.1",
        &HookLocalId::new("prompt-inject").expect("valid HookLocalId in test"),
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
            capability_view: None,
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
