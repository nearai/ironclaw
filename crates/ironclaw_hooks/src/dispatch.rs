//! Hook dispatcher — invokes the active hooks for a point with deterministic
//! ordering, panic isolation, timeout enforcement, slot poisoning on protocol
//! violation, and short-circuit semantics for gate phases.
//!
//! This crate ships the dispatcher contract; the Reborn-side middleware that
//! wires it into `LoopCapabilityPort` / `LoopPromptPort` / etc. lives in
//! `ironclaw_reborn::loop_driver_host` and lands in a follow-up slice.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Mutex;
use std::time::Duration;

use futures::FutureExt;

use crate::error::SanitizedReason;
use crate::failure_policy::{FailureCategory, FailureDisposition};
use crate::identity::HookId;
use crate::kinds::gate::{BeforeCapabilityHookDecision, GateDecisionInner};
use crate::kinds::mutator::HookPatch;
use crate::kinds::observer::ObserverFact;
use crate::ordering::HookOrderKey;
use crate::points::{BeforeCapabilityHookContext, BeforePromptHookContext, ObserverHookContext};
use crate::registry::{HookBinding, HookPointSpec, HookRegistry};
use crate::sink::{
    GateSinkState, ObserverHook, PrivilegedBeforeCapabilityHook, PrivilegedBeforePromptHook,
    RecordingGateSink, RecordingMutatorSink, RecordingObserverSink, RestrictedBeforeCapabilityHook,
    RestrictedBeforePromptHook,
};
use crate::trust::HookTrustClass;

/// Default per-hook wall-clock budget. Tunable per dispatcher.
pub const DEFAULT_HOOK_TIMEOUT: Duration = Duration::from_millis(50);

/// Tier-tagged trait object holding a `before_capability` hook implementation.
/// The variants make the trust tier explicit at the registration boundary so
/// the dispatcher routes through the correct sink trait.
pub enum BeforeCapabilityHookImpl {
    Privileged(Box<dyn PrivilegedBeforeCapabilityHook>),
    Restricted(Box<dyn RestrictedBeforeCapabilityHook>),
}

/// Tier-tagged trait object for a `before_prompt` mutator hook.
pub enum BeforePromptHookImpl {
    Privileged(Box<dyn PrivilegedBeforePromptHook>),
    Restricted(Box<dyn RestrictedBeforePromptHook>),
}

/// Tier-tagged trait object for an observer hook.
pub enum ObserverHookImpl {
    Any(Box<dyn ObserverHook>),
}

/// The composed outcome of dispatching `before_capability` against all active
/// hooks at the point.
#[derive(Debug)]
pub struct BeforeCapabilityDispatchOutcome {
    /// The composed decision after all hooks ran and short-circuits applied.
    pub decision: BeforeCapabilityHookDecision,
    /// Audit facts emitted by observers in the same dispatch. Always-run
    /// `Telemetry`-phase hooks land here even when an earlier `Gate`-phase
    /// hook denied.
    pub observer_facts: Vec<ObserverFact>,
    /// Per-hook failures encountered during this dispatch. Each entry tells
    /// downstream audit which hook misbehaved and how.
    pub failures: Vec<HookFailureRecord>,
}

/// Outcome of running a single `before_capability` hook to completion. The
/// `Pass` variant lets a hook explicitly state "no opinion" — the dispatcher
/// composes nothing for it, but does not treat the absence of a sink call
/// as a protocol violation. The `Decision` variant carries a minted decision
/// for the composer.
#[derive(Debug)]
pub(crate) enum GateHookOutcome {
    Pass,
    Decision(BeforeCapabilityHookDecision),
}

/// Per-hook record of misbehavior surfaced during a dispatch.
#[derive(Debug, Clone)]
pub struct HookFailureRecord {
    pub hook_id: HookId,
    pub category: FailureCategory,
    pub disposition: FailureDisposition,
    pub reason: SanitizedReason,
}

/// Composed outcome for `before_prompt`.
#[derive(Debug)]
pub struct BeforePromptDispatchOutcome {
    /// Patches that survived all checks, in deterministic order.
    pub patches: Vec<HookPatch>,
    pub observer_facts: Vec<ObserverFact>,
    pub failures: Vec<HookFailureRecord>,
}

/// Composed outcome for an observer dispatch.
#[derive(Debug)]
pub struct ObserverDispatchOutcome {
    pub facts: Vec<ObserverFact>,
    pub failures: Vec<HookFailureRecord>,
}

/// The dispatcher. Holds the registry plus the actual hook implementations.
///
/// The registry tracks bindings (id, version, trust class, phase) and is
/// serializable for checkpoint replay; the impls are runtime-only objects
/// resolved through a separate map.
pub struct HookDispatcher {
    registry: Mutex<HookRegistry>,
    before_capability: HashMap<HookId, BeforeCapabilityHookImpl>,
    before_prompt: HashMap<HookId, BeforePromptHookImpl>,
    observers: HashMap<HookId, ObserverHookImpl>,
    timeout: Duration,
}

impl HookDispatcher {
    pub fn new(registry: HookRegistry) -> Self {
        Self {
            registry: Mutex::new(registry),
            before_capability: HashMap::new(),
            before_prompt: HashMap::new(),
            observers: HashMap::new(),
            timeout: DEFAULT_HOOK_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Insert a new binding into the dispatcher's registry. Used by the
    /// [`crate::registrar::HookRegistrar`] to wire manifest entries into a
    /// live dispatcher. Returns the same errors as
    /// [`HookRegistry::insert`].
    pub fn insert_binding(&mut self, binding: HookBinding) -> Result<(), crate::error::HookError> {
        let mut registry = self.registry.lock().map_err(|_| {
            crate::error::HookError::RegistryConstruction(
                "hook registry mutex poisoned".to_string(),
            )
        })?;
        registry.insert(binding)
    }

    /// Register a hook implementation against an existing binding.
    pub fn install_before_capability(&mut self, hook_id: HookId, hook: BeforeCapabilityHookImpl) {
        self.before_capability.insert(hook_id, hook);
    }

    pub fn install_before_prompt(&mut self, hook_id: HookId, hook: BeforePromptHookImpl) {
        self.before_prompt.insert(hook_id, hook);
    }

    pub fn install_observer(&mut self, hook_id: HookId, hook: ObserverHookImpl) {
        self.observers.insert(hook_id, hook);
    }

    /// Dispatch `before_capability`. Hooks run in `(phase, priority, hook_id)`
    /// order. The first `Deny` short-circuits the gate phases; `Telemetry`
    /// phase observers always run.
    pub async fn dispatch_before_capability(
        &self,
        ctx: &BeforeCapabilityHookContext,
    ) -> BeforeCapabilityDispatchOutcome {
        let ordered = self.ordered_bindings(HookPointSpec::BeforeCapability);
        let mut composed = BeforeCapabilityHookDecision::allow();
        let mut observer_facts = Vec::new();
        let mut failures = Vec::new();
        let mut short_circuited = false;

        for (key, binding) in ordered {
            if short_circuited && !matches!(key.phase, crate::ordering::HookPhase::Telemetry) {
                continue;
            }
            let Some(hook) = self.before_capability.get(&binding.hook_id) else {
                // Binding present without an installed impl — record as
                // protocol violation and poison the slot.
                self.poison_with_failure(
                    binding.hook_id,
                    FailureCategory::Malformed,
                    binding.trust_class,
                    &crate::trust::DecisionKind::Gate,
                    "binding present without installed implementation",
                    &mut failures,
                );
                if !short_circuited {
                    composed = BeforeCapabilityHookDecision::deny(SanitizedReason::from_static(
                        "hook binding missing implementation",
                    ));
                    short_circuited = true;
                }
                continue;
            };

            let result = self.run_before_capability_hook(hook, &binding, ctx).await;
            match result {
                Ok(GateHookOutcome::Pass) => {
                    // Hook explicitly declared no opinion — contributes
                    // nothing to the composed decision.
                }
                Ok(GateHookOutcome::Decision(decision)) => {
                    composed = compose_gate_decision(composed, decision);
                    if !matches!(composed.inner(), GateDecisionInner::Allow) {
                        short_circuited = true;
                    }
                }
                Err(failure) => {
                    let restrictive = match failure.disposition {
                        FailureDisposition::FailClosed => {
                            Some(BeforeCapabilityHookDecision::deny(failure.reason.clone()))
                        }
                        FailureDisposition::FailIsolated => None,
                    };
                    failures.push(failure);
                    if let Some(deny) = restrictive {
                        composed = compose_gate_decision(composed, deny);
                        if !matches!(composed.inner(), GateDecisionInner::Allow) {
                            short_circuited = true;
                        }
                    }
                }
            }
        }

        // Drain observer-only telemetry hooks at this point (separate from
        // before_capability dispatch — observer impls are stored in
        // `observers` and resolved by their bindings in another map).
        let telemetry_outcome = self
            .dispatch_observer_at(HookPointSpec::AfterCapability, ctx.tenant_id.clone())
            .await;
        observer_facts.extend(telemetry_outcome.facts);
        failures.extend(telemetry_outcome.failures);

        BeforeCapabilityDispatchOutcome {
            decision: composed,
            observer_facts,
            failures,
        }
    }

    /// Dispatch `before_prompt`. All non-failing patches are returned in
    /// deterministic order. The dispatcher does not enforce the byte budget
    /// against `remaining_snippet_byte_budget` here — that check happens
    /// downstream in the prompt-bundle assembler.
    pub async fn dispatch_before_prompt(
        &self,
        ctx: &BeforePromptHookContext,
    ) -> BeforePromptDispatchOutcome {
        let ordered = self.ordered_bindings(HookPointSpec::BeforePrompt);
        let mut patches = Vec::new();
        let mut failures = Vec::new();

        for (_key, binding) in ordered {
            let Some(hook) = self.before_prompt.get(&binding.hook_id) else {
                self.poison_with_failure(
                    binding.hook_id,
                    FailureCategory::Malformed,
                    binding.trust_class,
                    &crate::trust::DecisionKind::Mutator,
                    "binding present without installed implementation",
                    &mut failures,
                );
                continue;
            };
            match self.run_before_prompt_hook(hook, &binding, ctx).await {
                Ok(mut emitted) => patches.append(&mut emitted),
                Err(failure) => failures.push(failure),
            }
        }

        BeforePromptDispatchOutcome {
            patches,
            observer_facts: Vec::new(),
            failures,
        }
    }

    /// Dispatch observer hooks at a given point. Called both directly and
    /// internally by `dispatch_before_capability` for the `AfterCapability`
    /// observers attached to the same dispatch slot.
    pub async fn dispatch_observer_at(
        &self,
        point: HookPointSpec,
        tenant: ironclaw_host_api::TenantId,
    ) -> ObserverDispatchOutcome {
        let ordered = self.ordered_bindings(point);
        let mut facts = Vec::new();
        let mut failures = Vec::new();
        let ctx = ObserverHookContext {
            tenant_id: tenant,
            observed_kind: match point {
                HookPointSpec::AfterModel => crate::points::observer::ObservedKind::AfterModel,
                HookPointSpec::AfterCapability => {
                    crate::points::observer::ObservedKind::AfterCapability
                }
                HookPointSpec::AfterCheckpoint => {
                    crate::points::observer::ObservedKind::AfterCheckpoint
                }
                _ => {
                    // Non-observer point passed in; return empty outcome and
                    // record a protocol violation against the dispatcher's own
                    // configuration (this is a bug in the caller).
                    return ObserverDispatchOutcome { facts, failures };
                }
            },
        };

        for (_key, binding) in ordered {
            let Some(hook) = self.observers.get(&binding.hook_id) else {
                self.poison_with_failure(
                    binding.hook_id,
                    FailureCategory::Malformed,
                    binding.trust_class,
                    &crate::trust::DecisionKind::Observer,
                    "binding present without installed implementation",
                    &mut failures,
                );
                continue;
            };
            match self.run_observer_hook(hook, &binding, &ctx).await {
                Ok(mut emitted) => facts.append(&mut emitted),
                Err(failure) => failures.push(failure),
            }
        }

        ObserverDispatchOutcome { facts, failures }
    }

    fn ordered_bindings(&self, point: HookPointSpec) -> Vec<(HookOrderKey, HookBinding)> {
        let registry = self.registry.lock().expect("hooks registry mutex poisoned");
        let mut out: Vec<_> = registry
            .active_at(point)
            .cloned()
            .map(|b| {
                let key =
                    HookOrderKey::new(b.phase, crate::ordering::HookPriority::DEFAULT, b.hook_id);
                (key, b)
            })
            .collect();
        out.sort_by_key(|(k, _)| *k);
        out
    }

    async fn run_before_capability_hook(
        &self,
        hook: &BeforeCapabilityHookImpl,
        binding: &HookBinding,
        ctx: &BeforeCapabilityHookContext,
    ) -> Result<GateHookOutcome, HookFailureRecord> {
        let timeout = self.timeout;
        let run = async {
            match hook {
                BeforeCapabilityHookImpl::Privileged(h) => {
                    let mut sink = RecordingGateSink::new();
                    AssertUnwindSafe(h.evaluate(ctx, &mut sink))
                        .catch_unwind()
                        .await
                        .map_err(|_| ())
                        .map(|()| sink.state)
                }
                BeforeCapabilityHookImpl::Restricted(h) => {
                    let mut sink = RecordingGateSink::new();
                    AssertUnwindSafe(h.evaluate(ctx, &mut sink))
                        .catch_unwind()
                        .await
                        .map_err(|_| ())
                        .map(|()| sink.state)
                }
            }
        };

        match tokio::time::timeout(timeout, run).await {
            Ok(Ok(GateSinkState::Decided(decision))) => Ok(GateHookOutcome::Decision(decision)),
            Ok(Ok(GateSinkState::Passed)) => Ok(GateHookOutcome::Pass),
            Ok(Ok(GateSinkState::Unset)) => {
                let failure = self.classify_failure(
                    binding,
                    FailureCategory::Malformed,
                    "hook completed without minting a decision",
                );
                Err(failure)
            }
            Ok(Err(())) => {
                let failure =
                    self.classify_failure(binding, FailureCategory::Panic, "hook panicked");
                Err(failure)
            }
            Err(_elapsed) => {
                let failure = self.classify_failure(
                    binding,
                    FailureCategory::Timeout,
                    "hook exceeded dispatch timeout",
                );
                Err(failure)
            }
        }
    }

    async fn run_before_prompt_hook(
        &self,
        hook: &BeforePromptHookImpl,
        binding: &HookBinding,
        ctx: &BeforePromptHookContext,
    ) -> Result<Vec<HookPatch>, HookFailureRecord> {
        let timeout = self.timeout;
        let run = async {
            match hook {
                BeforePromptHookImpl::Privileged(h) => {
                    let mut sink = RecordingMutatorSink::new(binding.trust_class);
                    AssertUnwindSafe(h.evaluate(ctx, &mut sink))
                        .catch_unwind()
                        .await
                        .map_err(|_| ())
                        .map(|()| sink.patches)
                }
                BeforePromptHookImpl::Restricted(h) => {
                    let mut sink = RecordingMutatorSink::new(binding.trust_class);
                    AssertUnwindSafe(h.evaluate(ctx, &mut sink))
                        .catch_unwind()
                        .await
                        .map_err(|_| ())
                        .map(|()| sink.patches)
                }
            }
        };

        match tokio::time::timeout(timeout, run).await {
            Ok(Ok(patches)) => Ok(patches),
            Ok(Err(())) => {
                Err(self.classify_failure(binding, FailureCategory::Panic, "hook panicked"))
            }
            Err(_elapsed) => Err(self.classify_failure(
                binding,
                FailureCategory::Timeout,
                "hook exceeded dispatch timeout",
            )),
        }
    }

    async fn run_observer_hook(
        &self,
        hook: &ObserverHookImpl,
        binding: &HookBinding,
        ctx: &ObserverHookContext,
    ) -> Result<Vec<ObserverFact>, HookFailureRecord> {
        let timeout = self.timeout;
        let run = async {
            match hook {
                ObserverHookImpl::Any(h) => {
                    let mut sink = RecordingObserverSink::new();
                    AssertUnwindSafe(h.observe(ctx, &mut sink))
                        .catch_unwind()
                        .await
                        .map_err(|_| ())
                        .map(|()| sink.facts)
                }
            }
        };

        match tokio::time::timeout(timeout, run).await {
            Ok(Ok(facts)) => Ok(facts),
            Ok(Err(())) => Err(self.classify_failure(
                binding,
                FailureCategory::Panic,
                "observer hook panicked",
            )),
            Err(_elapsed) => Err(self.classify_failure(
                binding,
                FailureCategory::Timeout,
                "observer hook exceeded dispatch timeout",
            )),
        }
    }

    fn classify_failure(
        &self,
        binding: &HookBinding,
        category: FailureCategory,
        reason: &'static str,
    ) -> HookFailureRecord {
        let kind = decision_kind_for(binding.point);
        let disposition = category.disposition_for(kind);
        // Poison the slot for the rest of the run.
        if let Ok(mut registry) = self.registry.lock() {
            registry.poison(binding.hook_id);
        }
        // Audit emission lives downstream; here we just record.
        tracing::warn!(
            hook_id = %binding.hook_id,
            category = ?category,
            disposition = ?disposition,
            "hook misbehavior recorded, slot poisoned"
        );
        HookFailureRecord {
            hook_id: binding.hook_id,
            category,
            disposition,
            reason: SanitizedReason::from_static(reason),
        }
    }

    fn poison_with_failure(
        &self,
        hook_id: HookId,
        category: FailureCategory,
        trust_class: HookTrustClass,
        kind: &crate::trust::DecisionKind,
        reason: &'static str,
        failures: &mut Vec<HookFailureRecord>,
    ) {
        let disposition = category.disposition_for(*kind);
        if let Ok(mut registry) = self.registry.lock() {
            registry.poison(hook_id);
        }
        tracing::warn!(
            %hook_id,
            ?category,
            ?trust_class,
            ?kind,
            "hook protocol violation, slot poisoned"
        );
        failures.push(HookFailureRecord {
            hook_id,
            category,
            disposition,
            reason: SanitizedReason::from_static(reason),
        });
    }
}

fn decision_kind_for(point: HookPointSpec) -> crate::trust::DecisionKind {
    match point {
        HookPointSpec::BeforeCapability => crate::trust::DecisionKind::Gate,
        HookPointSpec::BeforePrompt => crate::trust::DecisionKind::Mutator,
        HookPointSpec::AfterModel
        | HookPointSpec::AfterCapability
        | HookPointSpec::AfterCheckpoint => crate::trust::DecisionKind::Observer,
    }
}

/// Compose two gate decisions. The result is "the most restrictive of the
/// two." Order:
///
/// Deny > PauseAuth > PauseApproval > Allow
///
/// Pause variants compose by keeping the *first* observed pause (so the user
/// sees the first reason chronologically rather than the last). Deny always
/// wins.
fn compose_gate_decision(
    current: BeforeCapabilityHookDecision,
    new: BeforeCapabilityHookDecision,
) -> BeforeCapabilityHookDecision {
    use GateDecisionInner::*;
    match (current.inner(), new.inner()) {
        (Deny { .. }, _) => current,
        (_, Deny { .. }) => new,
        (PauseAuth { .. }, _) => current,
        (_, PauseAuth { .. }) => new,
        (PauseApproval { .. }, _) => current,
        (_, PauseApproval { .. }) => new,
        (Allow, Allow) => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ExtensionId, HookLocalId, HookVersion};
    use crate::kinds::mutator::PatchOrdinalHint;
    use crate::kinds::observer::NoteCategory;
    use crate::ordering::HookPhase;
    use crate::sink::{
        ObserverHook, ObserverSink, PrivilegedBeforeCapabilityHook, PrivilegedGateSink,
        RestrictedBeforeCapabilityHook, RestrictedBeforePromptHook, RestrictedGateSink,
        RestrictedMutatorSink,
    };
    use async_trait::async_trait;

    fn tenant() -> ironclaw_host_api::TenantId {
        ironclaw_host_api::TenantId::new("alpha").expect("tenant ok")
    }

    fn ext_hook_id(local: &str) -> HookId {
        HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId(local.to_string()),
            HookVersion::ONE,
        )
    }

    fn installed_binding(id: HookId, point: HookPointSpec, phase: HookPhase) -> HookBinding {
        HookBinding {
            hook_id: id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase,
            point,
            poisoned: false,
        }
    }

    fn ctx() -> BeforeCapabilityHookContext {
        BeforeCapabilityHookContext::new(tenant(), "cap.x".to_string(), [0u8; 32])
    }

    struct DenyingInstalledHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for DenyingInstalledHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.deny("blocked by extension");
        }
    }

    struct AllowingBuiltinHook;
    #[async_trait]
    impl PrivilegedBeforeCapabilityHook for AllowingBuiltinHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn PrivilegedGateSink,
        ) {
            sink.allow();
        }
    }

    struct PanickingHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for PanickingHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            _sink: &mut dyn RestrictedGateSink,
        ) {
            panic!("intentional panic in test hook");
        }
    }

    struct SlowHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for SlowHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            _sink: &mut dyn RestrictedGateSink,
        ) {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    struct EnvelopePatchHook;
    #[async_trait]
    impl RestrictedBeforePromptHook for EnvelopePatchHook {
        async fn evaluate(
            &self,
            _ctx: &BeforePromptHookContext,
            sink: &mut dyn RestrictedMutatorSink,
        ) {
            sink.add_envelope_snippet("safety".to_string(), PatchOrdinalHint::Last)
                .expect("ok");
        }
    }

    struct NotingObserver;
    #[async_trait]
    impl ObserverHook for NotingObserver {
        async fn observe(&self, _ctx: &ObserverHookContext, sink: &mut dyn ObserverSink) {
            sink.note(NoteCategory::HookFired, "fired");
        }
    }

    struct PassingInstalledHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for PassingInstalledHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.pass();
        }
    }

    struct SilentInstalledHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for SilentInstalledHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            _sink: &mut dyn RestrictedGateSink,
        ) {
            // Deliberately returns without calling any sink method.
        }
    }

    #[tokio::test]
    async fn pass_hook_does_not_short_circuit_allow() {
        let id = ext_hook_id("passes");
        let mut registry = HookRegistry::new();
        registry
            .insert(installed_binding(
                id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
            ))
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            id,
            BeforeCapabilityHookImpl::Restricted(Box::new(PassingInstalledHook)),
        );

        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(
            outcome.decision.permits(),
            "passing hook must not short-circuit the composed allow"
        );
        assert!(outcome.failures.is_empty(), "pass is not a failure");
    }

    #[tokio::test]
    async fn no_sink_call_is_still_malformed() {
        let id = ext_hook_id("silent");
        let mut registry = HookRegistry::new();
        registry
            .insert(installed_binding(
                id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
            ))
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            id,
            BeforeCapabilityHookImpl::Restricted(Box::new(SilentInstalledHook)),
        );

        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(
            !outcome.decision.permits(),
            "missing sink call must fail closed"
        );
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].category, FailureCategory::Malformed);
        assert!(
            dispatcher
                .registry
                .lock()
                .expect("registry")
                .is_poisoned(id)
        );
    }

    #[tokio::test]
    async fn install_only_no_bindings_allows() {
        let dispatcher = HookDispatcher::new(HookRegistry::new());
        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(outcome.decision.permits());
        assert!(outcome.failures.is_empty());
    }

    #[tokio::test]
    async fn installed_deny_short_circuits_to_deny() {
        let id = ext_hook_id("deny");
        let mut registry = HookRegistry::new();
        registry
            .insert(installed_binding(
                id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
            ))
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            id,
            BeforeCapabilityHookImpl::Restricted(Box::new(DenyingInstalledHook)),
        );

        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(!outcome.decision.permits());
    }

    #[tokio::test]
    async fn allow_then_deny_yields_deny() {
        let allow_id = HookId::for_builtin("test::allow", HookVersion::ONE);
        let deny_id = ext_hook_id("deny");

        let allow_binding = HookBinding {
            hook_id: allow_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Builtin,
            phase: HookPhase::Validation,
            point: HookPointSpec::BeforeCapability,
            poisoned: false,
        };
        let mut registry = HookRegistry::new();
        registry.insert(allow_binding).expect("ok");
        registry
            .insert(installed_binding(
                deny_id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
            ))
            .expect("ok");

        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            allow_id,
            BeforeCapabilityHookImpl::Privileged(Box::new(AllowingBuiltinHook)),
        );
        dispatcher.install_before_capability(
            deny_id,
            BeforeCapabilityHookImpl::Restricted(Box::new(DenyingInstalledHook)),
        );

        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(!outcome.decision.permits());
    }

    #[tokio::test]
    async fn panicking_hook_fails_closed_and_poisons_slot() {
        let id = ext_hook_id("panic");
        let mut registry = HookRegistry::new();
        registry
            .insert(installed_binding(
                id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
            ))
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            id,
            BeforeCapabilityHookImpl::Restricted(Box::new(PanickingHook)),
        );

        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(!outcome.decision.permits(), "panic should fail closed");
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].category, FailureCategory::Panic);
        assert!(
            dispatcher.registry.lock().unwrap().is_poisoned(id),
            "slot must be poisoned after panic"
        );
    }

    #[tokio::test]
    async fn slow_hook_times_out_and_fails_closed() {
        let id = ext_hook_id("slow");
        let mut registry = HookRegistry::new();
        registry
            .insert(installed_binding(
                id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
            ))
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry).with_timeout(Duration::from_millis(20));
        dispatcher.install_before_capability(
            id,
            BeforeCapabilityHookImpl::Restricted(Box::new(SlowHook)),
        );

        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(!outcome.decision.permits(), "timeout should fail closed");
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].category, FailureCategory::Timeout);
        assert!(dispatcher.registry.lock().unwrap().is_poisoned(id));
    }

    #[tokio::test]
    async fn missing_implementation_poisons_and_fails_closed() {
        let id = ext_hook_id("orphan");
        let mut registry = HookRegistry::new();
        registry
            .insert(installed_binding(
                id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
            ))
            .expect("ok");
        let dispatcher = HookDispatcher::new(registry);
        // Note: deliberately *not* installing the hook impl.

        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(!outcome.decision.permits());
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].category, FailureCategory::Malformed);
        assert!(dispatcher.registry.lock().unwrap().is_poisoned(id));
    }

    #[tokio::test]
    async fn before_prompt_collects_patches_in_order() {
        let id = ext_hook_id("envelope");
        let mut registry = HookRegistry::new();
        registry
            .insert(HookBinding {
                hook_id: id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Installed,
                phase: HookPhase::Policy,
                point: HookPointSpec::BeforePrompt,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_prompt(
            id,
            BeforePromptHookImpl::Restricted(Box::new(EnvelopePatchHook)),
        );

        let ctx = BeforePromptHookContext::new(tenant(), 4096);
        let outcome = dispatcher.dispatch_before_prompt(&ctx).await;
        assert_eq!(outcome.patches.len(), 1);
        assert!(outcome.failures.is_empty());
    }

    #[tokio::test]
    async fn observer_dispatch_collects_facts() {
        let id = HookId::for_builtin("test::observer", HookVersion::ONE);
        let mut registry = HookRegistry::new();
        registry
            .insert(HookBinding {
                hook_id: id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Builtin,
                phase: HookPhase::Telemetry,
                point: HookPointSpec::AfterModel,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_observer(id, ObserverHookImpl::Any(Box::new(NotingObserver)));

        let outcome = dispatcher
            .dispatch_observer_at(HookPointSpec::AfterModel, tenant())
            .await;
        assert_eq!(outcome.facts.len(), 1);
        assert!(outcome.failures.is_empty());
    }
}
