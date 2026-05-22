//! Capability-port middleware that runs `dispatch_before_capability` ahead of
//! every invocation and translates hook decisions into the existing
//! `CapabilityOutcome` vocabulary.
//!
//! Translation:
//!
//! - `GateDecisionInner::Allow` → forward to inner port unchanged.
//! - `GateDecisionInner::Deny` → return `CapabilityOutcome::Denied` with
//!   `CapabilityDeniedReasonKind::Unknown("hook_denied")` and the sanitized
//!   reason as `safe_summary`.
//! - `GateDecisionInner::PauseApproval` → mint an approval gate ref via the
//!   configured [`HookGateRefFactory`] and return
//!   `CapabilityOutcome::ApprovalRequired { gate_ref, safe_summary }`.
//! - `GateDecisionInner::PauseAuth` → mint an auth gate ref via the factory
//!   and return `CapabilityOutcome::AuthRequired { gate_ref, safe_summary }`.
//!
//! If the factory itself fails (e.g. the host's gate-router rejected the
//! mint), the middleware fails closed and surfaces the call as
//! `CapabilityOutcome::Denied` with a sanitized `hook_gate_ref_unavailable`
//! reason kind — better to refuse the call than route the loop through an
//! unresolvable suspension.
//!
//! Failure cases from the dispatcher (panic, timeout, missing impl) also map
//! to `Denied` per the [`crate::failure_policy`] rules.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::TenantId;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityDenied,
    CapabilityDeniedReasonKind, CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};

use crate::dispatch::{BeforeCapabilityDispatchOutcome, HookDispatcher};
use crate::kinds::gate::GateDecisionInner;
use crate::middleware::gate_ref::{FailClosedHookGateRefFactory, HookGateRefFactory};
use crate::middleware::resolver::{
    CapabilityInputResolver, CapabilityProviderResolver, NullCapabilityInputResolver,
    NullCapabilityProviderResolver,
};
use crate::points::{BeforeCapabilityHookContext, SanitizedArguments};

/// Wraps an inner `LoopCapabilityPort`, fires `before_capability` hooks ahead
/// of each invocation, and translates the dispatcher's composed decision into
/// the `CapabilityOutcome` vocabulary the loop driver already speaks.
pub struct HookedLoopCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    dispatcher: Arc<HookDispatcher>,
    tenant_id: TenantId,
    resolver: Arc<dyn CapabilityInputResolver>,
    provider_resolver: Arc<dyn CapabilityProviderResolver>,
    gate_ref_factory: Arc<dyn HookGateRefFactory>,
}

impl HookedLoopCapabilityPort {
    /// Construct a middleware with the bundled
    /// [`NullCapabilityInputResolver`]. Predicate evaluators that depend on
    /// argument contents (e.g., `ValueOrRateBound::NumericSum`) will fail
    /// closed; use [`Self::with_resolver`] to wire in a production resolver.
    pub fn new(
        inner: Arc<dyn LoopCapabilityPort>,
        dispatcher: Arc<HookDispatcher>,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            inner,
            dispatcher,
            tenant_id,
            resolver: Arc::new(NullCapabilityInputResolver),
            provider_resolver: Arc::new(NullCapabilityProviderResolver),
            // Default to fail-closed: minting a syntactically-valid but
            // router-unregistered ref is worse than refusing the suspension.
            // Callers must explicitly opt into UuidHookGateRefFactory for
            // tests/dev, or install a router-backed factory for production
            // (henrypark133 review Critical #3).
            gate_ref_factory: Arc::new(FailClosedHookGateRefFactory),
        }
    }

    /// Override the resolver used to surface sanitized arguments to hook
    /// predicates. Returns `self` so callers can chain after `new`.
    #[must_use]
    pub fn with_resolver(mut self, resolver: Arc<dyn CapabilityInputResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    /// Override the resolver used to populate
    /// [`crate::points::BeforeCapabilityHookContext::provider`] with the
    /// extension that owns the invoked capability. Required for
    /// `OwnCapabilities`-scoped Installed hooks to fire — without a
    /// production resolver the bundled [`NullCapabilityProviderResolver`]
    /// returns `None` and those hooks never see their own capabilities.
    #[must_use]
    pub fn with_provider_resolver(
        mut self,
        provider_resolver: Arc<dyn CapabilityProviderResolver>,
    ) -> Self {
        self.provider_resolver = provider_resolver;
        self
    }

    /// Override the gate-ref factory. Production code wires a factory that
    /// is bound to the current `LoopRunContext` and the host's approval-
    /// router so the resulting `ApprovalRequired` / `AuthRequired` outcomes
    /// resolve correctly. Tests and the foundation slice can rely on the
    /// default [`UuidHookGateRefFactory`].
    #[must_use]
    pub fn with_gate_ref_factory(mut self, factory: Arc<dyn HookGateRefFactory>) -> Self {
        self.gate_ref_factory = factory;
        self
    }

    async fn hook_context(
        &self,
        invocation: &CapabilityInvocation,
        provider: Option<ironclaw_host_api::ExtensionId>,
    ) -> BeforeCapabilityHookContext {
        let arguments = match self.resolver.resolve(invocation).await {
            Some(value) => SanitizedArguments::from_json(value),
            None => SanitizedArguments::unresolved(),
        };
        BeforeCapabilityHookContext::new(
            self.tenant_id.clone(),
            invocation.capability_id.to_string(),
            invocation_arguments_digest(invocation),
            arguments,
            provider,
        )
    }

    async fn run_dispatch(
        &self,
        invocation: &CapabilityInvocation,
        provider: Option<ironclaw_host_api::ExtensionId>,
    ) -> BeforeCapabilityDispatchOutcome {
        let ctx = self.hook_context(invocation, provider).await;
        self.dispatcher.dispatch_before_capability(&ctx).await
    }
}

#[async_trait]
impl LoopCapabilityPort for HookedLoopCapabilityPort {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        // Visible-surface queries don't go through hooks (the surface itself
        // is owned by profile-scoped filtering; hooks gate invocation, not
        // listing).
        self.inner.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let provider = self
            .provider_resolver
            .provider_for(&request.capability_id.to_string())
            .await;
        let outcome = self.run_dispatch(&request, provider.clone()).await;
        let result = match self.decision_to_outcome(&outcome).await {
            Some(translated) => Ok(translated),
            None => self.inner.invoke_capability(request).await,
        };
        // Fire AfterCapability observers regardless of whether the hook
        // short-circuited or the inner port ran. Observer-only point — no
        // gate decisions composed here. Telemetry must reflect both denied
        // and allowed invocations. The resolved provider is threaded so the
        // dispatcher can enforce `OwnCapabilities` scope on Installed
        // observers (serrrfirat finding #3).
        let _ = self
            .dispatcher
            .dispatch_observer_at_with_provider(
                crate::registry::HookPointSpec::AfterCapability,
                self.tenant_id.clone(),
                provider,
            )
            .await;
        result
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        // Each invocation runs its own hook pre-flight. Hooks can deny one
        // call in a batch without affecting others — the inner port still
        // executes the non-denied calls.
        let CapabilityBatchInvocation {
            invocations,
            stop_on_first_suspension,
        } = request;
        let mut outcomes = Vec::with_capacity(invocations.len());
        let mut stopped_on_suspension = false;
        for invocation in invocations {
            if stopped_on_suspension {
                break;
            }
            let provider = self
                .provider_resolver
                .provider_for(&invocation.capability_id.to_string())
                .await;
            let dispatch = self.run_dispatch(&invocation, provider.clone()).await;
            // Capture the inner result (Ok or Err) before dispatching
            // AfterCapability observers — propagating an error with `?`
            // here would skip observers for failed batch entries, which
            // is inconsistent with the single-invocation path and hides
            // failures from audit/telemetry (serrrfirat P2 #3, PR #3573).
            let invocation_result: Result<CapabilityOutcome, AgentLoopHostError> =
                match self.decision_to_outcome(&dispatch).await {
                    Some(translated) => Ok(translated),
                    None => self.inner.invoke_capability(invocation).await,
                };
            // Fire AfterCapability observers per batch entry, mirroring the
            // single-invocation path. The provider is resolved per-invocation
            // so the dispatcher can enforce `OwnCapabilities` scope on
            // Installed observers (serrrfirat finding #3).
            let _ = self
                .dispatcher
                .dispatch_observer_at_with_provider(
                    crate::registry::HookPointSpec::AfterCapability,
                    self.tenant_id.clone(),
                    provider,
                )
                .await;
            let outcome = invocation_result?;
            if outcome.is_suspension() && stop_on_first_suspension {
                stopped_on_suspension = true;
            }
            outcomes.push(outcome);
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

impl HookedLoopCapabilityPort {
    /// Translates a dispatcher outcome into a `CapabilityOutcome`. Returns
    /// `Some(outcome)` when the hook decision is restrictive (deny / pause /
    /// failure-closed), or `None` if the hooks allowed the call and the
    /// inner port should be consulted.
    ///
    /// This is async because pause-class decisions await the
    /// `HookGateRefFactory` to mint a real `LoopGateRef`. If the factory
    /// fails, the middleware falls back to `Denied` with a sanitized
    /// `hook_gate_ref_unavailable` reason.
    async fn decision_to_outcome(
        &self,
        dispatched: &BeforeCapabilityDispatchOutcome,
    ) -> Option<CapabilityOutcome> {
        match dispatched.decision.inner() {
            GateDecisionInner::Allow => None,
            GateDecisionInner::Deny { reason } => {
                Some(CapabilityOutcome::Denied(CapabilityDenied {
                    reason_kind: CapabilityDeniedReasonKind::unknown("hook_denied")
                        .expect("hook_denied is a valid loop-safe identifier"), // safety: literal ASCII identifier, validated by LoopGateRef constructor contract
                    safe_summary: reason.as_str().to_string(),
                }))
            }
            GateDecisionInner::PauseApproval { reason } => {
                match self
                    .gate_ref_factory
                    .mint_approval_ref(reason.as_str())
                    .await
                {
                    Ok(gate_ref) => Some(CapabilityOutcome::ApprovalRequired {
                        gate_ref,
                        safe_summary: reason.as_str().to_string(),
                    }),
                    Err(_) => Some(fail_closed_gate_ref_unavailable(reason.as_str())),
                }
            }
            GateDecisionInner::PauseAuth { reason } => {
                match self.gate_ref_factory.mint_auth_ref(reason.as_str()).await {
                    Ok(gate_ref) => Some(CapabilityOutcome::AuthRequired {
                        gate_ref,
                        safe_summary: reason.as_str().to_string(),
                    }),
                    Err(_) => Some(fail_closed_gate_ref_unavailable(reason.as_str())),
                }
            }
        }
    }
}

/// Fail-closed translation when the gate-ref factory cannot mint a ref for a
/// pause-class decision. The safe summary intentionally carries only the
/// hook's already-sanitized reason — the underlying host error is dropped to
/// avoid leaking internal gate-router state into model-visible output.
fn fail_closed_gate_ref_unavailable(sanitized_reason: &str) -> CapabilityOutcome {
    CapabilityOutcome::Denied(CapabilityDenied {
        reason_kind: CapabilityDeniedReasonKind::unknown("hook_gate_ref_unavailable")
            .expect("hook_gate_ref_unavailable is a valid loop-safe identifier"), // safety: literal ASCII identifier, validated by LoopGateRef constructor contract
        safe_summary: sanitized_reason.to_string(),
    })
}

/// Stable digest of capability arguments for hook context. The middleware
/// hashes the input-ref's underlying value so two invocations with identical
/// arguments produce the same digest, enabling repetition / rate-cap logic
/// without exposing raw arguments to hook code.
fn invocation_arguments_digest(invocation: &CapabilityInvocation) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    let cap = invocation.capability_id.to_string();
    hasher.update(&(cap.len() as u64).to_le_bytes());
    hasher.update(cap.as_bytes());
    // `as_str()` is the stable accessor for `CapabilityInputRef`. We avoid
    // `format!("{:?}", ...)` because `Debug` is not a stability contract —
    // a field rename or stdlib formatter change would silently shift the
    // digest, breaking any repetition-detection hook keyed on it.
    let input = invocation.input_ref.as_str();
    hasher.update(&(input.len() as u64).to_le_bytes());
    hasher.update(input.as_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
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
        CapabilityDescriptorView, CapabilityInputRef, CapabilityResultMessage,
        CapabilitySurfaceVersion,
    };
    use std::sync::Mutex;

    fn tenant() -> TenantId {
        TenantId::new("alpha").expect("ok")
    }

    struct AlwaysCompletedPort {
        calls: Mutex<Vec<CapabilityId>>,
    }

    impl AlwaysCompletedPort {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<CapabilityId> {
            self.calls.lock().expect("not poisoned").clone()
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

    fn invocation(capability: &str) -> CapabilityInvocation {
        CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("v1").expect("ok"),
            capability_id: CapabilityId::new(capability).expect("ok"),
            input_ref: CapabilityInputRef::new(format!("input:{capability}")).expect("ok"),
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
        let (dispatcher, _) =
            dispatcher_with_restricted_hook("pause-auth", Box::new(PauseAuthHook));
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
                unreachable!()
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
                owning_extension: None,
                scope: HookBindingScope::Global,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_observer_impl(
            observer_id,
            crate::dispatch::ObserverHookImpl::Any(Box::new(CountingObserver {
                seen: seen.clone(),
            })),
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
}
