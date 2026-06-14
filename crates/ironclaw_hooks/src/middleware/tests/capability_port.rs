use super::*;
use crate::dispatch::BeforeCapabilityHookImpl;
use crate::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
use crate::middleware::gate_ref::UuidHookGateRefFactory;
use crate::ordering::HookPhase;
use crate::ordering::HookPriority;
use crate::registry::{HookBinding, HookBindingScope, HookPointSpec, HookRegistry};
use crate::sink::{RestrictedBeforeCapabilityHook, RestrictedGateSink};
use crate::trust::HookTrustClass;
use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, RuntimeKind};
use ironclaw_turns::LoopResultRef;
use ironclaw_turns::run_profile::{
    CapabilityDescriptorView, CapabilityInputRef, CapabilityResultMessage, CapabilitySurfaceVersion,
};
use std::sync::Mutex;

fn tenant() -> TenantId {
    TenantId::new("alpha").expect("ok")
}

struct AlwaysCompletedPort {
    calls: Mutex<Vec<CapabilityId>>,
    /// Number of times `invoke_capability_batch` was called on the inner
    /// port. Distinct from `calls.len()`, which counts the per-entry
    /// `invoke_capability` invocations the batch impl makes underneath.
    batch_calls: Mutex<Vec<Vec<CapabilityId>>>,
}

impl AlwaysCompletedPort {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            batch_calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<CapabilityId> {
        self.calls.lock().expect("not poisoned").clone()
    }

    fn batch_calls(&self) -> Vec<Vec<CapabilityId>> {
        self.batch_calls.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl LoopCapabilityPort for AlwaysCompletedPort {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("v1").expect("ok"),
            descriptors: vec![CapabilityDescriptorView {
                capability_id: CapabilityId::new("cap.x").expect("ok"),
                provider: None,
                runtime: RuntimeKind::Wasm,
                safe_name: "cap.x".to_string(),
                safe_description: "test capability".to_string(),
                concurrency_hint: ironclaw_turns::run_profile::ConcurrencyHint::Exclusive,
                parameters_schema: serde_json::Value::Null,
            }],
        })
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.calls
            .lock()
            .expect("not poisoned")
            .push(request.capability_id.clone());
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: LoopResultRef::new(format!("result:{}", request.capability_id))
                .expect("ok"),
            safe_summary: format!("ran {}", request.capability_id),
            terminate_hint: false,
            byte_len: 0,
        }))
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.batch_calls.lock().expect("not poisoned").push(
            request
                .invocations
                .iter()
                .map(|i| i.capability_id.clone())
                .collect(),
        );
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

struct DenyingHook;
#[async_trait]
impl RestrictedBeforeCapabilityHook for DenyingHook {
    async fn evaluate(
        &self,
        _ctx: &BeforeCapabilityHookContext,
        sink: &mut dyn RestrictedGateSink,
    ) {
        sink.deny("blocked by extension policy");
    }
}

struct PauseApprovalHook;
#[async_trait]
impl RestrictedBeforeCapabilityHook for PauseApprovalHook {
    async fn evaluate(
        &self,
        _ctx: &BeforeCapabilityHookContext,
        sink: &mut dyn RestrictedGateSink,
    ) {
        sink.pause_approval("needs approval for this capability");
    }
}

struct PauseAuthHook;
#[async_trait]
impl RestrictedBeforeCapabilityHook for PauseAuthHook {
    async fn evaluate(
        &self,
        _ctx: &BeforeCapabilityHookContext,
        sink: &mut dyn RestrictedGateSink,
    ) {
        sink.pause_auth("needs auth for this capability");
    }
}

/// Records the digest a real `BeforeCapability` hook receives, then passes.
#[derive(Clone)]
struct CapturingBeforeCapabilityHook {
    captured: Arc<Mutex<Vec<[u8; 32]>>>,
}

impl CapturingBeforeCapabilityHook {
    fn new() -> Self {
        Self {
            captured: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn captured(&self) -> Vec<[u8; 32]> {
        self.captured.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl RestrictedBeforeCapabilityHook for CapturingBeforeCapabilityHook {
    async fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn RestrictedGateSink) {
        self.captured
            .lock()
            .expect("not poisoned")
            .push(ctx.arguments_digest);
        // Restricted hooks must make an explicit decision; no-op would fail closed.
        sink.pass();
    }
}

fn dispatcher_with_restricted_hook(
    local: &str,
    hook: Box<dyn RestrictedBeforeCapabilityHook>,
) -> (Arc<HookDispatcher>, HookId) {
    let hook_id = HookId::derive(
        &ExtensionId::new("ext").expect("valid ExtensionId in test"),
        "1.0",
        &HookLocalId::new(local).expect("valid HookLocalId in test"),
        HookVersion::ONE,
    );
    let binding = HookBinding {
        hook_id,
        hook_version: HookVersion::ONE,
        trust_class: HookTrustClass::Installed,
        phase: HookPhase::Policy,
        priority: HookPriority::DEFAULT,
        point: HookPointSpec::BeforeCapability,
        event_kind_filter: None,
        owning_extension: None,
        scope: HookBindingScope::Global,
        poisoned: false,
    };
    let mut registry = HookRegistry::new();
    registry.insert(binding).expect("ok");
    let mut dispatcher = HookDispatcher::new(registry);
    dispatcher.install_before_capability(hook_id, BeforeCapabilityHookImpl::Restricted(hook));
    (Arc::new(dispatcher), hook_id)
}

/// Test-only gate-ref factory that always errors. Used to exercise the
/// fail-closed path when the host's gate-router refuses to mint a ref.
struct FailingGateRefFactory;
#[async_trait]
impl crate::middleware::gate_ref::HookGateRefFactory for FailingGateRefFactory {
    async fn mint_approval_ref(
        &self,
        _reason: &str,
    ) -> Result<ironclaw_turns::LoopGateRef, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::Internal,
            "no router",
        ))
    }
    async fn mint_auth_ref(
        &self,
        _reason: &str,
    ) -> Result<ironclaw_turns::LoopGateRef, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::Internal,
            "no router",
        ))
    }
}

// Canonical digest fixture shared by helper-level and caller-driven pins.
// If the hex changes, audit every caller that keys on `arguments_digest`.
const SNAPSHOT_FIXTURE_DIGEST_HEX: &str =
    "4d0ab78e009b32615c2766bd1c26921bd59ef81b5741a75387707f82f0344315";

fn snapshot_fixture_invocation() -> CapabilityInvocation {
    CapabilityInvocation {
        surface_version: ironclaw_turns::run_profile::CapabilitySurfaceVersion::new("snapshot:v1")
            .expect("surface version literal is valid"),
        capability_id: CapabilityId::new("cap.snapshot.fixture")
            .expect("capability id literal is valid"),
        input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new(
            "input:cap.snapshot.fixture",
        )
        .expect("input ref literal is valid"),
        approval_resume: None,
        auth_resume: None,
    }
}

fn digest_hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn dispatcher_with_capturing_hook(hook: CapturingBeforeCapabilityHook) -> Arc<HookDispatcher> {
    let (dispatcher, _) = dispatcher_with_restricted_hook("capture-digest", Box::new(hook));
    dispatcher
}

fn assert_hook_observed_snapshot_digest(hook: &CapturingBeforeCapabilityHook, path: &str) {
    let captured = hook.captured();
    assert_eq!(
        captured.len(),
        1,
        "hook must observe exactly one BeforeCapability context"
    );
    assert_eq!(
        digest_hex(&captured[0]),
        SNAPSHOT_FIXTURE_DIGEST_HEX,
        "arguments_digest observed through {path} shifted; this is a \
             hook-visible wire-contract break"
    );
}

#[test]
fn invocation_arguments_digest_is_stable_for_known_inputs() {
    let invocation = snapshot_fixture_invocation();
    let digest = invocation_arguments_digest(&invocation);
    assert_eq!(
        digest_hex(&digest),
        SNAPSHOT_FIXTURE_DIGEST_HEX,
        "invocation_arguments_digest shifted for a fixed input — \
             this is a wire-contract break. See the stability-contract \
             rustdoc on `invocation_arguments_digest`."
    );
}

/// Caller-driven regression pin for the digest hook authors observe.
#[tokio::test]
async fn invoke_capability_arguments_digest_is_stable_at_middleware_boundary() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let hook = CapturingBeforeCapabilityHook::new();
    let dispatcher = dispatcher_with_capturing_hook(hook.clone());
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

    let outcome = wrapped
        .invoke_capability(snapshot_fixture_invocation())
        .await
        .expect("ok");

    // The capturing hook allows, so the invocation reaches the inner port.
    assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
    assert_eq!(
        inner.calls().len(),
        1,
        "allowed invocation must reach inner"
    );

    assert_hook_observed_snapshot_digest(&hook, "invoke_capability");
}

/// Batch-path variant of the caller-driven digest pin.
#[tokio::test]
async fn invoke_capability_batch_arguments_digest_is_stable_at_middleware_boundary() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let hook = CapturingBeforeCapabilityHook::new();
    let dispatcher = dispatcher_with_capturing_hook(hook.clone());
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

    // Two entries of the same fixture: exercises the merged-batch preflight
    // per-entry digest computation while also pinning the O(1)-batch
    // property — the inner port must see exactly one batched call.
    let batch = CapabilityBatchInvocation {
        invocations: vec![snapshot_fixture_invocation(), snapshot_fixture_invocation()],
        stop_on_first_suspension: false,
    };
    let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");

    assert_eq!(outcome.outcomes.len(), 2);
    assert!(
        outcome
            .outcomes
            .iter()
            .all(|o| matches!(o, CapabilityOutcome::Completed(_)))
    );

    // The hook runs once per entry, so both entries must be digested and
    // each must match the pinned snapshot — a batch-only digest regression
    // (e.g. dropping/swapping per-entry fields) would shift one of these.
    let captured = hook.captured();
    assert_eq!(
        captured.len(),
        2,
        "hook must observe one BeforeCapability context per batch entry"
    );
    for digest in &captured {
        assert_eq!(
            digest_hex(digest),
            SNAPSHOT_FIXTURE_DIGEST_HEX,
            "arguments_digest observed through invoke_capability_batch shifted; \
             this is a hook-visible wire-contract break"
        );
    }

    // The allowed batch must reach the inner port as a single batched call,
    // not degrade into N sequential invocations.
    let batch_calls = inner.batch_calls();
    assert_eq!(
        batch_calls.len(),
        1,
        "allowed batch must reach inner as exactly one batched call"
    );
    assert_eq!(batch_calls[0].len(), 2, "both entries batched together");
}

/// Distinct inputs must produce distinct digests (sanity check; the
/// snapshot test pins one specific point in input space, this widens
/// coverage to the structural property).
#[test]
fn invocation_arguments_digest_differs_for_different_input_refs() {
    let cap_id = CapabilityId::new("cap.x").expect("ok");
    let surface = ironclaw_turns::run_profile::CapabilitySurfaceVersion::new("v").expect("ok");
    let a = CapabilityInvocation {
        surface_version: surface.clone(),
        capability_id: cap_id.clone(),
        input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new("input:a").expect("ok"),
        approval_resume: None,
        auth_resume: None,
    };
    let b = CapabilityInvocation {
        surface_version: surface,
        capability_id: cap_id,
        input_ref: ironclaw_turns::run_profile::CapabilityInputRef::new("input:b").expect("ok"),
        approval_resume: None,
        auth_resume: None,
    };
    assert_ne!(
        invocation_arguments_digest(&a),
        invocation_arguments_digest(&b)
    );
}

/// A shared input ref must still hash differently per capability id.
#[test]
fn invocation_arguments_digest_differs_for_different_capability_ids() {
    let surface = ironclaw_turns::run_profile::CapabilitySurfaceVersion::new("v").expect("ok");
    let input_ref =
        ironclaw_turns::run_profile::CapabilityInputRef::new("input:shared").expect("ok");
    let a = CapabilityInvocation {
        surface_version: surface.clone(),
        capability_id: CapabilityId::new("cap.alpha").expect("ok"),
        input_ref: input_ref.clone(),
        approval_resume: None,
        auth_resume: None,
    };
    let b = CapabilityInvocation {
        surface_version: surface,
        capability_id: CapabilityId::new("cap.beta").expect("ok"),
        input_ref,
        approval_resume: None,
        auth_resume: None,
    };
    assert_ne!(
        invocation_arguments_digest(&a),
        invocation_arguments_digest(&b),
        "same input_ref with different capability_id must yield \
             distinct digests — the digest keys on the (capability_id, \
             input_ref) tuple, not input_ref alone."
    );
}

fn invocation(capability: &str) -> CapabilityInvocation {
    CapabilityInvocation {
        surface_version: CapabilitySurfaceVersion::new("v1").expect("ok"),
        capability_id: CapabilityId::new(capability).expect("ok"),
        input_ref: CapabilityInputRef::new(format!("input:{capability}")).expect("ok"),
        approval_resume: None,
        auth_resume: None,
    }
}

fn dispatcher_with_deny_hook() -> (Arc<HookDispatcher>, HookId) {
    let hook_id = HookId::derive(
        &ExtensionId::new("ext").expect("valid ExtensionId in test"),
        "1.0",
        &HookLocalId::new("deny").expect("valid HookLocalId in test"),
        HookVersion::ONE,
    );
    let binding = HookBinding {
        hook_id,
        hook_version: HookVersion::ONE,
        trust_class: HookTrustClass::Installed,
        phase: HookPhase::Policy,
        priority: HookPriority::DEFAULT,
        point: HookPointSpec::BeforeCapability,
        event_kind_filter: None,
        owning_extension: None,
        scope: HookBindingScope::Global,
        poisoned: false,
    };
    let mut registry = HookRegistry::new();
    registry.insert(binding).expect("ok");
    let mut dispatcher = HookDispatcher::new(registry);
    dispatcher.install_before_capability(
        hook_id,
        BeforeCapabilityHookImpl::Restricted(Box::new(DenyingHook)),
    );
    (Arc::new(dispatcher), hook_id)
}

#[tokio::test]
async fn deny_hook_short_circuits_invocation() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let (dispatcher, _) = dispatcher_with_deny_hook();
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

    let outcome = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    assert!(matches!(outcome, CapabilityOutcome::Denied(_)));
    assert!(
        inner.calls().is_empty(),
        "inner port must not be invoked when a hook denies"
    );
}

#[tokio::test]
async fn no_hooks_passes_through_to_inner() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

    let outcome = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
    assert_eq!(inner.calls().len(), 1);
}

#[tokio::test]
async fn batch_fires_dispatch_per_invocation() {
    // With the always-deny hook installed, every invocation in the batch
    // gets denied by hook dispatch and the inner port is never reached.
    // This verifies the wrapper's per-invocation dispatch loop, not just
    // the single-invocation path.
    let inner = Arc::new(AlwaysCompletedPort::new());
    let (dispatcher, _) = dispatcher_with_deny_hook();
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

    let batch = CapabilityBatchInvocation {
        invocations: vec![invocation("cap.alpha"), invocation("cap.beta")],
        stop_on_first_suspension: false,
    };
    let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
    assert_eq!(outcome.outcomes.len(), 2);
    assert!(inner.calls().is_empty(), "inner must not be invoked");
    for entry in &outcome.outcomes {
        assert!(matches!(entry, CapabilityOutcome::Denied(_)));
    }
}

#[tokio::test]
async fn pause_approval_decision_surfaces_as_approval_required() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let (dispatcher, _) =
        dispatcher_with_restricted_hook("pause-approval", Box::new(PauseApprovalHook));
    // Explicitly opt into the dev-only UUID gate-ref factory; the
    // middleware default is fail-closed (Critical #3).
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant())
        .with_gate_ref_factory(Arc::new(UuidHookGateRefFactory));

    let outcome = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    match outcome {
        CapabilityOutcome::ApprovalRequired {
            gate_ref,
            safe_summary,
            ..
        } => {
            assert!(gate_ref.as_str().starts_with("gate:hook-approval-"));
            assert_eq!(safe_summary, "needs approval for this capability");
        }
        other => panic!("expected ApprovalRequired, got {other:?}"),
    }
    assert!(inner.calls().is_empty(), "inner must not be invoked");
}

#[tokio::test]
async fn pause_auth_decision_surfaces_as_auth_required() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let (dispatcher, _) = dispatcher_with_restricted_hook("pause-auth", Box::new(PauseAuthHook));
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant())
        .with_gate_ref_factory(Arc::new(UuidHookGateRefFactory));

    let outcome = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    match outcome {
        CapabilityOutcome::AuthRequired {
            gate_ref,
            safe_summary,
        } => {
            assert!(gate_ref.as_str().starts_with("gate:hook-auth-"));
            assert_eq!(safe_summary, "needs auth for this capability");
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
    assert!(inner.calls().is_empty(), "inner must not be invoked");
}

#[tokio::test]
async fn gate_ref_factory_failure_falls_back_to_denied() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let (dispatcher, _) =
        dispatcher_with_restricted_hook("pause-approval-fail", Box::new(PauseApprovalHook));
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant())
        .with_gate_ref_factory(Arc::new(FailingGateRefFactory));

    let outcome = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    match outcome {
        CapabilityOutcome::Denied(denied) => {
            assert_eq!(
                denied.reason_kind,
                CapabilityDeniedReasonKind::unknown("hook_gate_ref_unavailable").expect("ok"),
            );
            // Sanitized hook reason is preserved; underlying error text
            // ("no router") must not leak.
            assert_eq!(denied.safe_summary, "needs approval for this capability");
        }
        other => panic!("expected Denied fallback, got {other:?}"),
    }
    assert!(inner.calls().is_empty(), "inner must not be invoked");
}

/// Companion to `gate_ref_factory_failure_falls_back_to_denied` covering
/// the `PauseAuth` arm. Deferred from the PR #3573 round-3 review.
/// When the gate-ref factory's `mint_auth_ref` errors, the middleware
/// must fail closed: surface `Denied` with the `hook_gate_ref_unavailable`
/// reason_kind, preserve the hook's sanitized summary, and never reach
/// the inner port.
#[tokio::test]
async fn gate_ref_factory_failure_for_pause_auth_falls_back_to_denied() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let (dispatcher, _) =
        dispatcher_with_restricted_hook("pause-auth-fail", Box::new(PauseAuthHook));
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant())
        .with_gate_ref_factory(Arc::new(FailingGateRefFactory));

    let outcome = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    match outcome {
        CapabilityOutcome::Denied(denied) => {
            assert_eq!(
                denied.reason_kind,
                CapabilityDeniedReasonKind::unknown("hook_gate_ref_unavailable").expect("ok"),
            );
            // Sanitized hook reason is preserved; underlying error text
            // ("no router") must not leak.
            assert_eq!(denied.safe_summary, "needs auth for this capability");
        }
        other => panic!("expected Denied fallback, got {other:?}"),
    }
    assert!(inner.calls().is_empty(), "inner must not be invoked");
}

/// serrrfirat P2 #3 on PR #3573: when an inner-port `invoke_capability`
/// in the batch loop returns `Err`, the previous implementation
/// propagated the error before dispatching `AfterCapability` observers.
/// This dropped failed batch entries from observer telemetry, in
/// contrast with the single-invocation path which dispatches observers
/// regardless. Pin the fixed behavior: the observer fires for the
/// failing entry, and the error still propagates.
#[tokio::test]
async fn batch_dispatches_after_capability_observers_on_inner_error() {
    use crate::points::ObserverHookContext;
    use crate::sink::{ObserverHook, ObserverSink};

    struct FailingPort;
    #[async_trait]
    impl LoopCapabilityPort for FailingPort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            unreachable!()
        }
        async fn invoke_capability(
            &self,
            _request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable,
                "inner port failed",
            ))
        }
        async fn invoke_capability_batch(
            &self,
            _request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            // After the batched-dispatch refactor, the wrapper forwards
            // hook-allowed invocations as a single batched call. Surface
            // the same Unavailable error here so the observer-on-error
            // contract from PR #3573 still has coverage.
            Err(AgentLoopHostError::new(
                ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable,
                "inner port failed",
            ))
        }
    }

    struct CountingObserver {
        seen: Arc<Mutex<u32>>,
    }
    #[async_trait]
    impl ObserverHook for CountingObserver {
        async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
            *self.seen.lock().expect("not poisoned") += 1;
        }
    }

    // Dispatcher with only an AfterCapability observer (no before-cap
    // gate → hooks allow → inner runs and fails).
    let seen = Arc::new(Mutex::new(0u32));
    let observer_id = HookId::for_builtin("test::after_cap_obs", HookVersion::ONE);
    let mut registry = HookRegistry::new();
    registry
        .insert(HookBinding {
            hook_id: observer_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Builtin,
            phase: HookPhase::Telemetry,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::AfterCapability,
            event_kind_filter: None,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        })
        .expect("ok");
    let mut dispatcher = HookDispatcher::new(registry);
    dispatcher.install_observer_impl(
        observer_id,
        crate::dispatch::ObserverHookImpl::Any(Box::new(CountingObserver { seen: seen.clone() })),
    );

    let wrapped =
        HookedLoopCapabilityPort::new(Arc::new(FailingPort), Arc::new(dispatcher), tenant());

    let batch = CapabilityBatchInvocation {
        invocations: vec![invocation("cap.x")],
        stop_on_first_suspension: false,
    };
    let err = wrapped
        .invoke_capability_batch(batch)
        .await
        .expect_err("inner err propagates");
    assert_eq!(
        err.kind,
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable
    );
    assert_eq!(
        *seen.lock().expect("not poisoned"),
        1,
        "AfterCapability observer must fire even when inner port errors \
             so failed batch entries are visible to telemetry"
    );
}

#[tokio::test]
async fn batch_passes_through_when_no_hooks() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

    let batch = CapabilityBatchInvocation {
        invocations: vec![invocation("cap.alpha"), invocation("cap.beta")],
        stop_on_first_suspension: false,
    };
    let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
    assert_eq!(outcome.outcomes.len(), 2);
    assert_eq!(inner.calls().len(), 2);
    for entry in &outcome.outcomes {
        assert!(matches!(entry, CapabilityOutcome::Completed(_)));
    }
}

// ── Batched-dispatch regression coverage ───────────────────────────────
//
// The middleware previously degraded `invoke_capability_batch` into N
// sequential `invoke_capability` calls whenever any hook was registered,
// wiping out the O(1)-batch property that the inner port relies on for
// bulk-dispatch performance. The tests below pin the restored behavior
// (PR #3573 deferred refactor).

/// When NO hook denies, the wrapper must call the inner port's
/// `invoke_capability_batch` exactly once, with every invocation in a
/// single payload — not N times via `invoke_capability`.
#[tokio::test]
async fn batch_invocation_remains_batched_when_no_hooks_deny() {
    struct AllowingHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for AllowingHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.pass();
        }
    }

    let inner = Arc::new(AlwaysCompletedPort::new());
    let (dispatcher, _) = dispatcher_with_restricted_hook("allowing", Box::new(AllowingHook));
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

    let batch = CapabilityBatchInvocation {
        invocations: vec![
            invocation("cap.alpha"),
            invocation("cap.beta"),
            invocation("cap.gamma"),
        ],
        stop_on_first_suspension: false,
    };
    let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
    assert_eq!(outcome.outcomes.len(), 3);
    // The critical perf invariant: ONE batched call, not three sequential.
    let batch_calls = inner.batch_calls();
    assert_eq!(
        batch_calls.len(),
        1,
        "inner port must see exactly one batched call when hooks allow all entries; \
             saw {} batched calls (sequential degradation)",
        batch_calls.len()
    );
    assert_eq!(
        batch_calls[0].len(),
        3,
        "all three entries batched together"
    );
}

/// Partial denial: a hook denies some entries; the surviving entries
/// must still be forwarded as a SINGLE batched call to the inner port,
/// and the merged outcomes must be in original index order.
#[tokio::test]
async fn batch_invocation_filters_denied_entries_and_preserves_index_mapping() {
    /// Denies only the capability whose name matches `target`.
    struct SelectiveDenyHook {
        target: &'static str,
    }
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for SelectiveDenyHook {
        async fn evaluate(
            &self,
            ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            if ctx.capability_name == self.target {
                sink.deny("selective deny");
            } else {
                sink.pass();
            }
        }
    }

    let inner = Arc::new(AlwaysCompletedPort::new());
    let (dispatcher, _) = dispatcher_with_restricted_hook(
        "selective",
        Box::new(SelectiveDenyHook { target: "cap.beta" }),
    );
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

    let batch = CapabilityBatchInvocation {
        invocations: vec![
            invocation("cap.alpha"),
            invocation("cap.beta"),
            invocation("cap.gamma"),
        ],
        stop_on_first_suspension: false,
    };
    let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
    assert_eq!(outcome.outcomes.len(), 3);
    // Index 0 and 2 forwarded to inner; index 1 short-circuited to Denied.
    assert!(matches!(
        outcome.outcomes[0],
        CapabilityOutcome::Completed(_)
    ));
    assert!(matches!(outcome.outcomes[1], CapabilityOutcome::Denied(_)));
    assert!(matches!(
        outcome.outcomes[2],
        CapabilityOutcome::Completed(_)
    ));

    let batch_calls = inner.batch_calls();
    assert_eq!(
        batch_calls.len(),
        1,
        "remaining entries must be forwarded in a single batched call"
    );
    assert_eq!(
        batch_calls[0].len(),
        2,
        "only the two non-denied entries reach the inner port"
    );
    // Order must match the original (allowed) order: alpha then gamma.
    assert_eq!(batch_calls[0][0].as_str(), "cap.alpha");
    assert_eq!(batch_calls[0][1].as_str(), "cap.gamma");
}

/// Even though the inner port is called only ONCE, `AfterCapability`
/// observers must fire per-entry against the merged outcome vec — the
/// per-entry telemetry contract from PR #3573 (serrrfirat finding #3)
/// is independent of the inner-port call topology.
#[tokio::test]
async fn batch_invocation_dispatches_after_capability_observer_per_entry() {
    use crate::points::ObserverHookContext;
    use crate::sink::{ObserverHook, ObserverSink};

    struct CountingObserver {
        seen: Arc<Mutex<u32>>,
    }
    #[async_trait]
    impl ObserverHook for CountingObserver {
        async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
            *self.seen.lock().expect("not poisoned") += 1;
        }
    }

    let seen = Arc::new(Mutex::new(0u32));
    let observer_id = HookId::for_builtin("test::after_cap_per_entry", HookVersion::ONE);
    let mut registry = HookRegistry::new();
    registry
        .insert(HookBinding {
            hook_id: observer_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Builtin,
            phase: HookPhase::Telemetry,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::AfterCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
            event_kind_filter: None,
            poisoned: false,
        })
        .expect("ok");
    let mut dispatcher = HookDispatcher::new(registry);
    dispatcher.install_observer_impl(
        observer_id,
        crate::dispatch::ObserverHookImpl::Any(Box::new(CountingObserver { seen: seen.clone() })),
    );

    let inner = Arc::new(AlwaysCompletedPort::new());
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), Arc::new(dispatcher), tenant());

    let batch = CapabilityBatchInvocation {
        invocations: vec![
            invocation("cap.alpha"),
            invocation("cap.beta"),
            invocation("cap.gamma"),
        ],
        stop_on_first_suspension: false,
    };
    let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
    assert_eq!(outcome.outcomes.len(), 3);
    assert_eq!(
        inner.batch_calls().len(),
        1,
        "inner port called exactly once (batched)"
    );
    assert_eq!(
        *seen.lock().expect("not poisoned"),
        3,
        "AfterCapability observer must fire per merged entry (3 entries) \
             even though the inner port was batched into a single call"
    );
}

// ── C3 regression: provider resolver populates hook context ────────────

use crate::middleware::resolver::CapabilityProviderResolver;
use crate::points::BeforeCapabilityHookContext as HookCtxForTest;
use ironclaw_host_api::ExtensionId as HostExtensionId;

/// Resolver that records every capability_id it was queried for and
/// returns a fixed provider for each call.
struct RecordingProviderResolver {
    provider: HostExtensionId,
    queried: Mutex<Vec<String>>,
}

#[async_trait]
impl CapabilityProviderResolver for RecordingProviderResolver {
    async fn provider_for(&self, capability_id: &str) -> Option<HostExtensionId> {
        self.queried
            .lock()
            .expect("recording resolver not poisoned")
            .push(capability_id.to_string());
        Some(self.provider.clone())
    }
}

/// Hook that records the provider observed in `ctx.provider`. Always
/// passes (no opinion) so the inner port still runs.
struct ProviderRecordingHook {
    observed: Arc<Mutex<Option<Option<HostExtensionId>>>>,
}

#[async_trait]
impl RestrictedBeforeCapabilityHook for ProviderRecordingHook {
    async fn evaluate(&self, ctx: &HookCtxForTest, sink: &mut dyn RestrictedGateSink) {
        *self.observed.lock().expect("observed mutex ok") = Some(ctx.provider.clone());
        sink.pass();
    }
}

#[tokio::test]
async fn provider_resolver_populates_hook_context() {
    let provider = HostExtensionId::new("ext-resolver-test").expect("valid ext id");
    let resolver = Arc::new(RecordingProviderResolver {
        provider: provider.clone(),
        queried: Mutex::new(Vec::new()),
    });

    // Use Global scope so the hook fires; we're testing the *context*,
    // not the scope filter.
    let hook_id = HookId::derive(
        &ExtensionId::new("ext").expect("valid ExtensionId in test"),
        "1.0",
        &HookLocalId::new("recording").expect("valid HookLocalId in test"),
        HookVersion::ONE,
    );
    let observed = Arc::new(Mutex::new(None));
    let hook = ProviderRecordingHook {
        observed: Arc::clone(&observed),
    };
    let mut dispatcher = HookDispatcher::new(HookRegistry::new());
    dispatcher
        .install_installed_before_capability(
            hook_id,
            HookPhase::Policy,
            HostExtensionId::new("ext-resolver-test").expect("valid"),
            crate::registry::HookBindingScope::Global,
            Box::new(hook),
        )
        .expect("install ok");

    let inner = Arc::new(AlwaysCompletedPort::new());
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), Arc::new(dispatcher), tenant())
        .with_provider_resolver(Arc::clone(&resolver) as Arc<_>);

    let _ = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    let observed = observed.lock().expect("observed mutex ok").clone();
    assert_eq!(
        observed,
        Some(Some(provider.clone())),
        "hook ctx must carry the resolver-supplied provider"
    );

    let queried = resolver.queried.lock().expect("queries").clone();
    assert_eq!(queried, vec!["cap.x".to_string()]);
}

// ── Lazy capability-input resolution (PR #3573 HIGH follow-up) ──────────
//
// The middleware must not pay the cost of resolving capability inputs
// when no active hook would read them. For inputs that are file blobs
// or other expensive sources, eager resolution can be wasteful or
// fatal. These tests pin the lazy-probe contract end-to-end via
// `invoke_capability`, not just through the helper, so that future
// changes to the dispatch path can't reintroduce eager resolution
// without also breaking a test.

use crate::evaluator::PredicateEvaluator;
use crate::installed_hook::PredicateBackedBeforeCapabilityHook;
use crate::predicate::{
    CapabilityPredicate, HookPredicateSpec, OnExceededAction, ValueOrRateBound,
};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering as AtomicOrdering};

/// Resolver that records how many times `resolve` and `size_hint` were
/// called and returns a configurable value/size. Used to prove the
/// middleware skips work when no predicate needs input.
struct ProbingResolver {
    resolve_calls: AtomicU32,
    size_hint_calls: AtomicU32,
    size: Option<u64>,
    value: serde_json::Value,
}

impl ProbingResolver {
    fn new(value: serde_json::Value) -> Self {
        Self {
            resolve_calls: AtomicU32::new(0),
            size_hint_calls: AtomicU32::new(0),
            size: None,
            value,
        }
    }

    fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    fn resolve_calls(&self) -> u32 {
        self.resolve_calls.load(AtomicOrdering::SeqCst)
    }

    fn size_hint_calls(&self) -> u32 {
        self.size_hint_calls.load(AtomicOrdering::SeqCst)
    }
}

#[async_trait]
impl CapabilityInputResolver for ProbingResolver {
    async fn resolve(&self, _invocation: &CapabilityInvocation) -> Option<serde_json::Value> {
        self.resolve_calls.fetch_add(1, AtomicOrdering::SeqCst);
        Some(self.value.clone())
    }

    async fn size_hint(&self, _invocation: &CapabilityInvocation) -> Option<u64> {
        self.size_hint_calls.fetch_add(1, AtomicOrdering::SeqCst);
        self.size
    }
}

fn install_predicate_hook(
    dispatcher: &mut HookDispatcher,
    local: &str,
    spec: HookPredicateSpec,
    evaluator: Arc<PredicateEvaluator>,
) -> HookId {
    let hook_id = HookId::derive(
        &ExtensionId::new("ext").expect("ext literal is valid"),
        "1.0",
        &HookLocalId::new(local).expect("local id literal is valid"),
        HookVersion::ONE,
    );
    let hook = PredicateBackedBeforeCapabilityHook::new(hook_id, spec, evaluator);
    dispatcher
        .install_installed_before_capability(
            hook_id,
            HookPhase::Policy,
            ironclaw_host_api::ExtensionId::new("ext-test").expect("valid"),
            HookBindingScope::Global,
            Box::new(hook),
        )
        .expect("install ok");
    hook_id
}

/// Pin lazy-resolution: when every active predicate-backed hook gates
/// only on invocation count (no input access), the dispatcher must
/// not consult the resolver at all. Instrument the resolver and
/// drive an `invoke_capability` through the middleware to assert
/// zero reads.
#[tokio::test]
async fn dispatch_skips_input_resolution_when_no_predicate_needs_input() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let mut dispatcher = HookDispatcher::new(HookRegistry::new());
    let evaluator = Arc::new(PredicateEvaluator::new());
    // InvocationCount: pure rate counter, never reads input.
    let spec = HookPredicateSpec::RateOrValueCap {
        when: CapabilityPredicate::Always,
        bound: ValueOrRateBound::InvocationCount {
            max: 100,
            window: "1h".to_string(),
        },
        on_exceeded: OnExceededAction::Deny {
            reason: "rate".to_string(),
        },
    };
    install_predicate_hook(&mut dispatcher, "rate-only", spec, evaluator);

    let resolver = Arc::new(ProbingResolver::new(serde_json::json!({"amount": 1})));
    let wrapped = HookedLoopCapabilityPort::new(inner, Arc::new(dispatcher), tenant())
        .with_resolver(Arc::clone(&resolver) as Arc<_>);

    let _ = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    assert_eq!(
        resolver.resolve_calls(),
        0,
        "no active predicate needs the capability input — resolver must not be consulted"
    );
    assert_eq!(
        resolver.size_hint_calls(),
        0,
        "size_hint must also be skipped when no hook needs input"
    );
}

/// Pin the inverse: when a `NumericSum` predicate is active, the
/// resolver IS consulted (so the predicate can read the field). This
/// is the regression complement to the skip test — together they
/// pin the lazy-probe behavior in both directions.
#[tokio::test]
async fn dispatch_reads_input_when_numericsum_predicate_active() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let mut dispatcher = HookDispatcher::new(HookRegistry::new());
    let evaluator = Arc::new(PredicateEvaluator::new());
    let spec = HookPredicateSpec::RateOrValueCap {
        when: CapabilityPredicate::Always,
        bound: ValueOrRateBound::NumericSum {
            max: "1000".to_string(),
            field: "amount".to_string(),
            window: "1h".to_string(),
        },
        on_exceeded: OnExceededAction::Deny {
            reason: "value".to_string(),
        },
    };
    install_predicate_hook(&mut dispatcher, "value-cap", spec, evaluator);

    let resolver = Arc::new(ProbingResolver::new(serde_json::json!({"amount": 1})));
    let wrapped = HookedLoopCapabilityPort::new(inner, Arc::new(dispatcher), tenant())
        .with_resolver(Arc::clone(&resolver) as Arc<_>);

    let _ = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    assert_eq!(
        resolver.resolve_calls(),
        1,
        "NumericSum predicate needs input; resolver must be consulted exactly once"
    );
}

/// Pin the streaming size guard: when `size_hint` reports a value
/// above `MAX_PREDICATE_INPUT_BYTES`, the middleware must fail
/// closed without calling `resolve`. The predicate evaluator then
/// treats the input as unresolved and the dispatch denies per the
/// `on_exceeded` action — the inner port is never invoked.
#[tokio::test]
async fn dispatch_fails_closed_when_input_exceeds_max_bytes() {
    let inner = Arc::new(AlwaysCompletedPort::new());
    let mut dispatcher = HookDispatcher::new(HookRegistry::new());
    let evaluator = Arc::new(PredicateEvaluator::new());
    let spec = HookPredicateSpec::RateOrValueCap {
        when: CapabilityPredicate::Always,
        bound: ValueOrRateBound::NumericSum {
            max: "1000".to_string(),
            field: "amount".to_string(),
            window: "1h".to_string(),
        },
        on_exceeded: OnExceededAction::Deny {
            reason: "value".to_string(),
        },
    };
    install_predicate_hook(&mut dispatcher, "value-cap-oversized", spec, evaluator);

    // Size hint reports a value above the cap — resolve must be skipped.
    let resolver = Arc::new(
        ProbingResolver::new(serde_json::json!({"amount": 1}))
            .with_size(MAX_PREDICATE_INPUT_BYTES + 1),
    );
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), Arc::new(dispatcher), tenant())
        .with_resolver(Arc::clone(&resolver) as Arc<_>);

    let outcome = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    assert!(
        matches!(outcome, CapabilityOutcome::Denied(_)),
        "oversized input must fail closed to a Denied outcome (got {outcome:?})"
    );
    assert_eq!(
        resolver.size_hint_calls(),
        1,
        "size_hint must be consulted exactly once"
    );
    assert_eq!(
        resolver.resolve_calls(),
        0,
        "resolve must be skipped when size_hint exceeds the cap"
    );
    assert!(
        inner.calls().is_empty(),
        "inner port must not be invoked when the hook denies"
    );
}

/// Pin the streaming order-of-operations: `size_hint` is consulted
/// **before** `resolve` for input-reading predicates. Without this
/// ordering, the middleware would materialize the value first and
/// only then notice it was too large — defeating the purpose of the
/// streaming pre-check.
#[tokio::test]
async fn dispatch_streams_size_check_before_full_materialization() {
    // Resolver that records the order of its calls. If `resolve` is
    // ever observed before `size_hint`, the test fails.
    struct OrderingResolver {
        sequence: AtomicU64,
        size_hint_seq: AtomicU64,
        resolve_seq: AtomicU64,
        size: u64,
    }
    #[async_trait]
    impl CapabilityInputResolver for OrderingResolver {
        async fn resolve(&self, _invocation: &CapabilityInvocation) -> Option<serde_json::Value> {
            let s = self.sequence.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            self.resolve_seq.store(s, AtomicOrdering::SeqCst);
            Some(serde_json::json!({"amount": 1}))
        }
        async fn size_hint(&self, _invocation: &CapabilityInvocation) -> Option<u64> {
            let s = self.sequence.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            self.size_hint_seq.store(s, AtomicOrdering::SeqCst);
            Some(self.size)
        }
    }

    // Use a size just below the cap so resolve still runs after the
    // pre-check — we want to assert *ordering*, not skip.
    let resolver = Arc::new(OrderingResolver {
        sequence: AtomicU64::new(0),
        size_hint_seq: AtomicU64::new(0),
        resolve_seq: AtomicU64::new(0),
        size: MAX_PREDICATE_INPUT_BYTES - 1,
    });

    let inner = Arc::new(AlwaysCompletedPort::new());
    let mut dispatcher = HookDispatcher::new(HookRegistry::new());
    let evaluator = Arc::new(PredicateEvaluator::new());
    let spec = HookPredicateSpec::RateOrValueCap {
        when: CapabilityPredicate::Always,
        bound: ValueOrRateBound::NumericSum {
            max: "1000".to_string(),
            field: "amount".to_string(),
            window: "1h".to_string(),
        },
        on_exceeded: OnExceededAction::Deny {
            reason: "value".to_string(),
        },
    };
    install_predicate_hook(&mut dispatcher, "ordering", spec, evaluator);

    let wrapped = HookedLoopCapabilityPort::new(inner, Arc::new(dispatcher), tenant())
        .with_resolver(Arc::clone(&resolver) as Arc<_>);

    let _ = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    let size_hint_seq = resolver.size_hint_seq.load(AtomicOrdering::SeqCst);
    let resolve_seq = resolver.resolve_seq.load(AtomicOrdering::SeqCst);
    assert!(size_hint_seq > 0, "size_hint must have been called");
    assert!(resolve_seq > 0, "resolve must have been called");
    assert!(
        size_hint_seq < resolve_seq,
        "size_hint (seq {size_hint_seq}) must be consulted before resolve (seq {resolve_seq}) so oversized inputs never get materialized"
    );
}

/// henrypark133 L2 on PR #3913: when several BeforeCapability hooks
/// share a scope+provider, `before_capability_needs_input` must
/// return `true` as soon as ANY active binding's hook reports
/// `needs_input() = true`. The short-circuit doesn't depend on which
/// hook the trait's default lives on, so a mixed pair (one false,
/// one true) must still cause `resolve_arguments` to be called by
/// the middleware. Drive the whole path via `invoke_capability` and
/// an instrumented resolver to confirm the call actually happens.
#[tokio::test]
async fn before_capability_needs_input_returns_true_when_any_active_binding_needs_input() {
    struct NoInputHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for NoInputHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.pass();
        }
        fn needs_input(&self) -> bool {
            false
        }
    }

    struct NeedsInputHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for NeedsInputHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.pass();
        }
        fn needs_input(&self) -> bool {
            true
        }
    }

    // Two BeforeCapability bindings on the same scope. One opts
    // out of input access; the other opts in. The dispatcher's
    // input-needed probe must short-circuit to true on the second
    // binding regardless of registration order.
    let no_input_id = HookId::derive(
        &ExtensionId::new("ext").expect("valid"),
        "1.0",
        &HookLocalId::new("no-input").expect("valid"),
        HookVersion::ONE,
    );
    let needs_input_id = HookId::derive(
        &ExtensionId::new("ext").expect("valid"),
        "1.0",
        &HookLocalId::new("needs-input").expect("valid"),
        HookVersion::ONE,
    );
    let mut registry = HookRegistry::new();
    registry
        .insert(HookBinding {
            hook_id: no_input_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
            event_kind_filter: None,
            poisoned: false,
        })
        .expect("insert no_input");
    registry
        .insert(HookBinding {
            hook_id: needs_input_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
            event_kind_filter: None,
            poisoned: false,
        })
        .expect("insert needs_input");
    let mut dispatcher = HookDispatcher::new(registry);
    dispatcher.install_before_capability(
        no_input_id,
        BeforeCapabilityHookImpl::Restricted(Box::new(NoInputHook)),
    );
    dispatcher.install_before_capability(
        needs_input_id,
        BeforeCapabilityHookImpl::Restricted(Box::new(NeedsInputHook)),
    );

    // First half: probe directly via the dispatcher's
    // `before_capability_needs_input` API.
    let dispatcher = Arc::new(dispatcher);
    assert!(
        dispatcher.before_capability_needs_input(None),
        "with a mixed pair (one needs_input=false, one needs_input=true) the \
             dispatcher must return true so the middleware materializes input",
    );

    // Second half: drive `invoke_capability` end-to-end with an
    // instrumented resolver. The middleware must consult `resolve`
    // because at least one active binding needs input — confirming
    // the short-circuit is wired all the way through the call site,
    // not just the helper.
    let resolver = Arc::new(ProbingResolver::new(serde_json::json!({"amount": 1})));
    let inner = Arc::new(AlwaysCompletedPort::new());
    let wrapped = HookedLoopCapabilityPort::new(inner, dispatcher, tenant())
        .with_resolver(Arc::clone(&resolver) as Arc<_>);

    let _ = wrapped
        .invoke_capability(invocation("cap.x"))
        .await
        .expect("ok");

    assert_eq!(
        resolver.resolve_calls(),
        1,
        "resolver must be called exactly once when at least one active \
             binding's hook reports needs_input()=true",
    );
}

/// henrypark133 M1 on PR #3911: in the merged-batch path, when an
/// allowed (Pending) entry precedes a hook-resolved suspension entry
/// and `stop_on_first_suspension` is true, the merge loop must still
/// fire the `AfterCapability` observer for the suspension slot. The
/// pre-fix loop initialized `stopped_on_suspension` from
/// `stopped_in_preflight` and broke after the first iteration —
/// dropping the observer for the trailing Resolved suspension slot
/// and losing it from `outcomes` entirely. This pins the fixed
/// behavior: every slot's observer fires and every Resolved slot
/// surfaces in `outcomes`, even when `stop_on_first_suspension` is
/// set and the suspension appears after an allowed entry.
#[tokio::test]
async fn batch_invocation_fires_observer_for_hook_suspended_entry_after_allowed_entry_with_stop_on_first_suspension()
 {
    use crate::points::ObserverHookContext;
    use crate::sink::{ObserverHook, ObserverSink};

    /// Allows `cap.alpha`, pauses `cap.beta` for approval. The
    /// mixed-batch scenario the merge loop has to handle correctly.
    struct SelectivePauseHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for SelectivePauseHook {
        async fn evaluate(
            &self,
            ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            if ctx.capability_name == "cap.beta" {
                sink.pause_approval("needs approval");
            } else {
                sink.pass();
            }
        }
    }

    struct CountingObserver {
        seen: Arc<Mutex<u32>>,
    }
    #[async_trait]
    impl ObserverHook for CountingObserver {
        async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
            *self.seen.lock().expect("not poisoned") += 1;
        }
    }

    // Set up a dispatcher with both the gating hook AND an
    // AfterCapability observer so we can count per-entry observer
    // dispatch.
    let gating_id = HookId::derive(
        &ExtensionId::new("ext").expect("valid ExtensionId in test"),
        "1.0",
        &HookLocalId::new("selective-pause").expect("valid HookLocalId in test"),
        HookVersion::ONE,
    );
    let gating_binding = HookBinding {
        hook_id: gating_id,
        hook_version: HookVersion::ONE,
        trust_class: HookTrustClass::Installed,
        phase: HookPhase::Policy,
        priority: HookPriority::DEFAULT,
        point: HookPointSpec::BeforeCapability,
        owning_extension: None,
        scope: HookBindingScope::Global,
        event_kind_filter: None,
        poisoned: false,
    };

    let observer_id = HookId::for_builtin("test::after_cap_mixed_susp", HookVersion::ONE);
    let observer_binding = HookBinding {
        hook_id: observer_id,
        hook_version: HookVersion::ONE,
        trust_class: HookTrustClass::Builtin,
        phase: HookPhase::Telemetry,
        priority: HookPriority::DEFAULT,
        point: HookPointSpec::AfterCapability,
        owning_extension: None,
        scope: HookBindingScope::Global,
        event_kind_filter: None,
        poisoned: false,
    };

    let mut registry = HookRegistry::new();
    registry.insert(gating_binding).expect("insert gating");
    registry.insert(observer_binding).expect("insert observer");
    let mut dispatcher = HookDispatcher::new(registry);
    dispatcher.install_before_capability(
        gating_id,
        BeforeCapabilityHookImpl::Restricted(Box::new(SelectivePauseHook)),
    );
    let seen = Arc::new(Mutex::new(0u32));
    dispatcher.install_observer_impl(
        observer_id,
        crate::dispatch::ObserverHookImpl::Any(Box::new(CountingObserver { seen: seen.clone() })),
    );

    let inner = Arc::new(AlwaysCompletedPort::new());
    let wrapped = HookedLoopCapabilityPort::new(inner.clone(), Arc::new(dispatcher), tenant())
        .with_gate_ref_factory(Arc::new(UuidHookGateRefFactory));

    // alpha (hook-allowed → Pending) precedes beta (hook-suspension
    // → Resolved). `stop_on_first_suspension` is true so the bug
    // path: the merge loop sees `stopped_in_preflight = true` from
    // Phase 1 (beta was the suspension trigger), pushes alpha, then
    // breaks before reaching beta's slot — dropping beta's observer
    // and outcome.
    let batch = CapabilityBatchInvocation {
        invocations: vec![invocation("cap.alpha"), invocation("cap.beta")],
        stop_on_first_suspension: true,
    };
    let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");

    assert_eq!(
        outcome.outcomes.len(),
        2,
        "merged outcomes must contain both entries: the hook-allowed alpha \
             (from inner) and the hook-suspended beta (from hook resolution). \
             Pre-fix this dropped beta to a length of 1.",
    );
    assert!(
        matches!(outcome.outcomes[0], CapabilityOutcome::Completed(_)),
        "alpha was hook-allowed and inner produced a Completed outcome",
    );
    assert!(
        matches!(
            outcome.outcomes[1],
            CapabilityOutcome::ApprovalRequired { .. }
        ),
        "beta was hook-resolved to ApprovalRequired",
    );
    assert!(
        outcome.stopped_on_suspension,
        "stop_on_first_suspension was set and a suspension surfaced",
    );
    assert_eq!(
        *seen.lock().expect("not poisoned"),
        2,
        "AfterCapability observer must fire for BOTH entries — the \
             hook-allowed alpha AND the hook-resolved suspension beta. \
             Pre-fix the merge loop broke before firing beta's observer.",
    );
    // No inner work for beta — the hook short-circuited it. The
    // inner test port's `invoke_capability_batch` impl delegates
    // per-entry to `invoke_capability`, so `calls()` counts the
    // entries the inner port saw; only alpha should appear.
    let calls = inner.calls();
    assert_eq!(
        calls.len(),
        1,
        "only alpha must reach the inner port; beta was hook-suspended",
    );
    assert_eq!(calls[0].as_str(), "cap.alpha");
    let batch_calls = inner.batch_calls();
    assert_eq!(
        batch_calls.len(),
        1,
        "the inner port must be batched-called exactly once with just alpha",
    );
    assert_eq!(batch_calls[0].len(), 1);
    assert_eq!(batch_calls[0][0].as_str(), "cap.alpha");
}
