//! Hook dispatcher — invokes the active hooks for a point with deterministic
//! ordering, panic isolation, timeout enforcement, slot poisoning on protocol
//! violation, and short-circuit semantics for gate phases.
//!
//! This crate ships the dispatcher contract; the Reborn-side middleware that
//! wires it into `LoopCapabilityPort` / `LoopPromptPort` / etc. lives in
//! `ironclaw_reborn::loop_driver_host` and lands in a follow-up slice.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use ironclaw_turns::run_profile::{HookDecisionSummary, HookMilestoneSink, LoopHostMilestoneKind};

use crate::error::SanitizedReason;
use crate::failure_policy::{FailureCategory, FailureDisposition};
use crate::identity::HookId;
use crate::identity::HookVersion;
use crate::kinds::gate::{BeforeCapabilityHookDecision, GateDecisionInner};
use crate::kinds::mutator::HookPatch;
use crate::kinds::observer::ObserverFact;
use crate::ordering::{HookOrderKey, HookPhase, HookPriority};
use crate::points::{BeforeCapabilityHookContext, BeforePromptHookContext, ObserverHookContext};
use crate::registry::{HookBinding, HookBindingScope, HookPointSpec, HookRegistry};
use crate::sink::{
    GateSinkState, ObserverHook, PrivilegedBeforeCapabilityHook, PrivilegedBeforePromptHook,
    RecordingGateSink, RecordingMutatorSink, RecordingObserverSink, RestrictedBeforeCapabilityHook,
    RestrictedBeforePromptHook,
};
use crate::telemetry;
use crate::trust::HookTrustClass;
use crate::wasm::{
    WasmBeforeCapabilityHook, WasmBeforePromptHook, WasmHookFailure, WasmObserverHook,
};

/// Default per-hook wall-clock budget. Tunable per dispatcher.
pub const DEFAULT_HOOK_TIMEOUT: Duration = Duration::from_millis(50);

/// Tier-tagged trait object holding a `before_capability` hook implementation.
/// The variants make the trust tier explicit at the registration boundary so
/// the dispatcher routes through the correct sink trait.
///
/// This type is deliberately `pub(crate)`. The only way to introduce a
/// `Privileged` impl into the dispatcher is through one of the
/// `install_builtin_*` / `install_trusted_*` constructors on
/// [`HookDispatcher`], which always construct the matching binding with a
/// `Builtin` or `Trusted` trust class. This is what makes the
/// "Installed cannot Allow" property a *type-level* invariant: no external
/// caller can pair `HookTrustClass::Installed` with
/// `BeforeCapabilityHookImpl::Privileged` because they cannot construct
/// `Privileged` at all.
pub(crate) enum BeforeCapabilityHookImpl {
    Privileged(Box<dyn PrivilegedBeforeCapabilityHook>),
    Restricted(Box<dyn RestrictedBeforeCapabilityHook>),
    RestrictedWasm(WasmBeforeCapabilityHook),
}

impl BeforeCapabilityHookImpl {
    /// Delegates to the inner hook's `needs_input()`. Used by the
    /// dispatch middleware to skip eager capability-input resolution when
    /// no active hook will consult the arguments.
    pub(crate) fn needs_input(&self) -> bool {
        match self {
            BeforeCapabilityHookImpl::Privileged(h) => h.needs_input(),
            BeforeCapabilityHookImpl::Restricted(h) => h.needs_input(),
            // WASM hooks: conservatively assume they read arguments. The
            // manifest does not currently expose a `needs_input` capability
            // bit, so eager input resolution is required for correctness.
            // Once a manifest-declared input descriptor lands, this can be
            // refined.
            BeforeCapabilityHookImpl::RestrictedWasm(_) => true,
        }
    }
}

/// Tier-tagged trait object for a `before_prompt` mutator hook. Same trust
/// rationale as [`BeforeCapabilityHookImpl`] — sealed to this crate.
pub(crate) enum BeforePromptHookImpl {
    Privileged(Box<dyn PrivilegedBeforePromptHook>),
    Restricted(Box<dyn RestrictedBeforePromptHook>),
    RestrictedWasm(WasmBeforePromptHook),
}

/// Tier-tagged trait object for an observer hook. Sealed to this crate for
/// API symmetry; observers have the same trait surface for every tier but the
/// registry still tracks trust_class for audit attribution.
pub(crate) enum ObserverHookImpl {
    Any(Box<dyn ObserverHook>),
    Wasm(WasmObserverHook),
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
    Decision {
        decision: BeforeCapabilityHookDecision,
        /// Free-form audit-only reason set by the hook via
        /// [`crate::sink::PrivilegedGateSink::record_audit_reason`] /
        /// [`crate::sink::RestrictedGateSink::record_audit_reason`]. The
        /// model-visible reason inside `decision` is the closed-vocab
        /// label; this carries the manifest-supplied context for audit/SSE.
        audit_reason: Option<String>,
    },
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
    milestone_sink: Option<Arc<dyn HookMilestoneSink>>,
}

impl HookDispatcher {
    /// Construct a bare dispatcher.
    ///
    /// **Internal:** outside this crate, use [`HookDispatcherBuilder::new`]
    /// followed by [`HookDispatcherBuilder::build_arc`]. The builder is the
    /// only public path to construction; this constructor remains visible to
    /// the crate so middleware unit tests can compose dispatchers directly
    /// without paying for an `Arc` wrap.
    pub(crate) fn new(registry: HookRegistry) -> Self {
        Self {
            registry: Mutex::new(registry),
            before_capability: HashMap::new(),
            before_prompt: HashMap::new(),
            observers: HashMap::new(),
            timeout: DEFAULT_HOOK_TIMEOUT,
            milestone_sink: None,
        }
    }

    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Attach a [`HookMilestoneSink`] to this dispatcher. When set, the
    /// dispatcher emits `HookDispatched`, `HookDecisionEmitted`, and
    /// `HookFailed` kinds into the sink as hooks run. Default (no sink)
    /// preserves the pre-telemetry behavior.
    ///
    /// Milestone payloads carry stringified hook ids, point names, and
    /// failure labels — never raw hook implementation state or user-facing
    /// content. See [`crate::telemetry`] for the conversion helpers.
    ///
    /// Because the dispatcher is typically held behind an `Arc` after it has
    /// been installed into the Reborn factory, callers must wire the sink
    /// *before* wrapping the dispatcher in `Arc`. This is the documented
    /// composition order: build dispatcher, set sink, wrap in `Arc`, install
    /// into the factory via `with_hook_dispatcher`. The sink should be a
    /// [`ironclaw_turns::run_profile::RunScopedHookMilestoneSink`] (or
    /// equivalent adapter) that injects run-context before forwarding to the
    /// host's `LoopHostMilestoneSink`.
    pub(crate) fn with_milestone_sink(mut self, sink: Arc<dyn HookMilestoneSink>) -> Self {
        self.milestone_sink = Some(sink);
        self
    }

    async fn emit_milestone(&self, kind: LoopHostMilestoneKind) {
        if let Some(sink) = &self.milestone_sink {
            sink.publish_hook_milestone(kind).await;
        }
    }

    /// Test-only accessor for inspecting registered bindings. The registry
    /// itself remains private to enforce the dispatcher-as-authority model;
    /// this hatch only exists so other crates' tests (e.g. the registrar's)
    /// can assert on binding shape after install.
    ///
    /// **Gated behind `cfg(test)` or the `test-support` feature.** Exposing
    /// the inner `&Mutex<HookRegistry>` in production would let any holder of
    /// an `Arc<HookDispatcher>` lock it and call mutators such as
    /// `HookRegistry::poison`, bypassing the builder's post-`build_arc`
    /// immutability guarantee and the registrar's grant/cap path. Production
    /// callers needing read-only inspection should use
    /// [`Self::active_bindings_snapshot`] instead.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn registry_for_test(&self) -> &Mutex<HookRegistry> {
        &self.registry
    }

    /// Returns `true` when at least one active (non-poisoned)
    /// `BeforeCapability` hook would read the capability input
    /// (`ctx.arguments`) during dispatch, given the resolved `provider`
    /// for the invocation.
    ///
    /// Used by the dispatch middleware as a lazy-resolution probe:
    /// expensive input materialization is skipped entirely when this
    /// returns `false`, which is the common case for purely rate-limited
    /// or name-matched specs (review of PR #3573).
    ///
    /// Scope filtering matches the rule the dispatcher itself applies in
    /// [`Self::dispatch_before_capability`]: a binding whose scope
    /// rejects the current provider is inert for the invocation and is
    /// not counted toward the input-needed decision. Bindings with no
    /// installed impl are also skipped — they would short-circuit as a
    /// dispatch-time protocol violation and never reach `evaluate`.
    pub fn before_capability_needs_input(
        &self,
        provider: Option<&ironclaw_host_api::ExtensionId>,
    ) -> bool {
        let bindings = self.active_bindings_snapshot(HookPointSpec::BeforeCapability);
        for binding in bindings {
            if !binding
                .scope
                .permits(binding.owning_extension.as_ref(), provider)
            {
                continue;
            }
            let Some(impl_) = self.before_capability.get(&binding.hook_id) else {
                continue;
            };
            if impl_.needs_input() {
                return true;
            }
        }
        false
    }

    /// Read-only snapshot of currently-active (not poisoned) bindings at a
    /// given point. Safe to expose in production: callers receive an owned
    /// `Vec<HookBinding>` rather than a handle to the registry mutex, so
    /// they cannot mutate dispatcher state.
    pub fn active_bindings_snapshot(&self, point: HookPointSpec) -> Vec<HookBinding> {
        let registry = self.registry.lock().expect("hooks registry mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
        registry.active_at(point).cloned().collect()
    }

    /// Validate that every hook id pinned by a checkpoint still resolves in
    /// the active registry before replay proceeds. This catches extension
    /// version drift and WASM module substitution as an explicit
    /// `UnknownHook` instead of silently dropping a previously-audited hook.
    pub fn validate_checkpoint_hook_ids_for_replay(
        &self,
        hook_ids: &[HookId],
    ) -> Result<(), crate::error::HookError> {
        let registry = self.registry.lock().map_err(|_| {
            crate::error::HookError::RegistryConstruction(
                "hook registry mutex poisoned".to_string(),
            )
        })?;
        for hook_id in hook_ids {
            if !registry.contains_hook(*hook_id) {
                return Err(crate::error::HookError::UnknownHook(*hook_id));
            }
        }
        Ok(())
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

    /// Count of bindings whose `owning_extension` matches `extension`
    /// across every attach point. Used by the registrar to enforce the
    /// per-extension cap cumulatively across multiple `install()` calls.
    pub(crate) fn count_bindings_for_extension(
        &self,
        extension: &ironclaw_host_api::ExtensionId,
    ) -> usize {
        let registry = self.registry.lock().expect("hook registry mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
        registry.count_for_extension(extension)
    }

    /// Same as [`Self::count_bindings_for_extension`] but restricted to
    /// one attach point. Powers the D4 (per-extension-per-kind) cap.
    pub(crate) fn count_bindings_for_extension_at(
        &self,
        extension: &ironclaw_host_api::ExtensionId,
        point: HookPointSpec,
    ) -> usize {
        let registry = self.registry.lock().expect("hook registry mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
        registry.count_for_extension_at(extension, point)
    }

    /// Update an already-inserted binding's `priority` field. The registrar
    /// uses this to apply the manifest-declared priority after the
    /// tier-specific installer creates the binding with the default. No-op
    /// when the hook is not registered.
    ///
    /// Why this is split from the installer: the tier-specific installers
    /// take `(hook_id, phase, ...)` but not priority — adding priority to
    /// every installer signature would churn ~40 call sites for a value
    /// that's only ever non-default on the manifest path. The registrar is
    /// the only caller that knows the manifest priority, so it sets it
    /// post-insert.
    pub(crate) fn set_binding_priority(&mut self, hook_id: HookId, priority: HookPriority) {
        let mut registry = self.registry.lock().expect("hook registry mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
        registry.set_priority(hook_id, priority);
    }

    /// Internal: register a hook implementation against an existing binding.
    /// All public installers route through this; the public surface enforces
    /// trust-tier × impl-tier pairing at the type level.
    pub(crate) fn install_before_capability(
        &mut self,
        hook_id: HookId,
        hook: BeforeCapabilityHookImpl,
    ) {
        self.before_capability.insert(hook_id, hook);
    }

    pub(crate) fn install_before_prompt(&mut self, hook_id: HookId, hook: BeforePromptHookImpl) {
        self.before_prompt.insert(hook_id, hook);
    }

    pub(crate) fn install_observer_impl(&mut self, hook_id: HookId, hook: ObserverHookImpl) {
        self.observers.insert(hook_id, hook);
    }

    // ── Tier-specific public installers for before_capability ───────────────
    //
    // Each installer builds the `HookBinding` with the correct trust class and
    // routes the impl into the matching enum variant. There is no public path
    // that pairs an `Installed` binding with a `Privileged` impl: the
    // `Privileged` variant is `pub(crate)` and cannot be constructed outside
    // this crate.

    /// Install a `Builtin`-tier `before_capability` hook. Builtins may mint
    /// any decision (including `allow`).
    pub(crate) fn install_builtin_before_capability(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        hook: Box<dyn PrivilegedBeforeCapabilityHook>,
    ) -> Result<(), crate::error::HookError> {
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Builtin,
            phase,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_before_capability(hook_id, BeforeCapabilityHookImpl::Privileged(hook));
        Ok(())
    }

    /// Install a `Trusted`-tier `before_capability` hook. Trusted hooks may
    /// mint any decision but cannot register at runtime-class phases.
    pub(crate) fn install_trusted_before_capability(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        hook: Box<dyn PrivilegedBeforeCapabilityHook>,
    ) -> Result<(), crate::error::HookError> {
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Trusted,
            phase,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_before_capability(hook_id, BeforeCapabilityHookImpl::Privileged(hook));
        Ok(())
    }

    /// Install an `Installed`-tier `before_capability` hook. The impl trait is
    /// `RestrictedBeforeCapabilityHook`, whose sink cannot mint `allow` — this
    /// makes "Installed cannot Allow" a type-level fact.
    ///
    /// `owning_extension` is the [`ironclaw_host_api::ExtensionId`] of the
    /// extension that authored the hook (from the manifest), and `scope`
    /// reflects the manifest-declared scope. The dispatcher consults both at
    /// invocation time to filter out hooks that shouldn't fire against the
    /// current capability's provider.
    pub(crate) fn install_installed_before_capability(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        owning_extension: ironclaw_host_api::ExtensionId,
        scope: HookBindingScope,
        hook: Box<dyn RestrictedBeforeCapabilityHook>,
    ) -> Result<(), crate::error::HookError> {
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            owning_extension: Some(owning_extension),
            scope,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_before_capability(hook_id, BeforeCapabilityHookImpl::Restricted(hook));
        Ok(())
    }

    /// Install an Installed-tier WASM `before_capability` hook through the
    /// restricted sink surface. WASM can deny or pause, but cannot mint Allow.
    pub(crate) fn install_installed_wasm_before_capability(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        owning_extension: ironclaw_host_api::ExtensionId,
        scope: HookBindingScope,
        hook: WasmBeforeCapabilityHook,
    ) -> Result<(), crate::error::HookError> {
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            owning_extension: Some(owning_extension),
            scope,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_before_capability(hook_id, BeforeCapabilityHookImpl::RestrictedWasm(hook));
        Ok(())
    }

    // ── Tier-specific public installers for before_prompt ───────────────────

    pub(crate) fn install_builtin_before_prompt(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        hook: Box<dyn PrivilegedBeforePromptHook>,
    ) -> Result<(), crate::error::HookError> {
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Builtin,
            phase,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforePrompt,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_before_prompt(hook_id, BeforePromptHookImpl::Privileged(hook));
        Ok(())
    }

    pub(crate) fn install_trusted_before_prompt(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        hook: Box<dyn PrivilegedBeforePromptHook>,
    ) -> Result<(), crate::error::HookError> {
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Trusted,
            phase,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforePrompt,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_before_prompt(hook_id, BeforePromptHookImpl::Privileged(hook));
        Ok(())
    }

    pub(crate) fn install_installed_before_prompt(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        owning_extension: ironclaw_host_api::ExtensionId,
        scope: HookBindingScope,
        hook: Box<dyn RestrictedBeforePromptHook>,
    ) -> Result<(), crate::error::HookError> {
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforePrompt,
            owning_extension: Some(owning_extension),
            scope,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_before_prompt(hook_id, BeforePromptHookImpl::Restricted(hook));
        Ok(())
    }

    /// Install an Installed-tier WASM `before_prompt` hook through the
    /// restricted mutator surface.
    pub(crate) fn install_installed_wasm_before_prompt(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        owning_extension: ironclaw_host_api::ExtensionId,
        scope: HookBindingScope,
        hook: WasmBeforePromptHook,
    ) -> Result<(), crate::error::HookError> {
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase,
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforePrompt,
            owning_extension: Some(owning_extension),
            scope,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_before_prompt(hook_id, BeforePromptHookImpl::RestrictedWasm(hook));
        Ok(())
    }

    // ── Observer installers ────────────────────────────────────────────────
    //
    // Observers share a single trait surface across all tiers, but the
    // registry still records the trust class for audit attribution. The
    // generic `install_observer` accepts an explicit trust class; the
    // tier-specific helpers make the common case ergonomic.

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn install_observer(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        point: HookPointSpec,
        trust_class: HookTrustClass,
        owning_extension: Option<ironclaw_host_api::ExtensionId>,
        scope: HookBindingScope,
        hook: Box<dyn ObserverHook>,
    ) -> Result<(), crate::error::HookError> {
        // Reject non-observer points at install time. Previously this path
        // accepted any `HookPointSpec`, populated the binding registry, but
        // only inserted into the observer map — so a `BeforeCapability`
        // observer installation would later cause `dispatch_before_capability`
        // to see a binding without a gate impl, poison the slot, and
        // fail-close the capability. Catch the misuse at install time
        // instead. (serrrfirat P2 #2 on PR #3573.)
        match point {
            HookPointSpec::AfterModel
            | HookPointSpec::AfterCapability
            | HookPointSpec::AfterCheckpoint => {}
            HookPointSpec::BeforeCapability | HookPointSpec::BeforePrompt => {
                return Err(crate::error::HookError::RegistryConstruction(format!(
                    "observer hooks cannot be installed at {point:?}; that \
                     point dispatches gate/mutator implementations, not \
                     observers"
                )));
            }
        }
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class,
            phase,
            priority: HookPriority::DEFAULT,
            point,
            owning_extension,
            scope,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_observer_impl(hook_id, ObserverHookImpl::Any(hook));
        Ok(())
    }

    pub(crate) fn install_builtin_observer(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        point: HookPointSpec,
        hook: Box<dyn ObserverHook>,
    ) -> Result<(), crate::error::HookError> {
        self.install_observer(
            hook_id,
            phase,
            point,
            HookTrustClass::Builtin,
            None,
            HookBindingScope::Global,
            hook,
        )
    }

    pub(crate) fn install_trusted_observer(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        point: HookPointSpec,
        hook: Box<dyn ObserverHook>,
    ) -> Result<(), crate::error::HookError> {
        self.install_observer(
            hook_id,
            phase,
            point,
            HookTrustClass::Trusted,
            None,
            HookBindingScope::Global,
            hook,
        )
    }

    pub(crate) fn install_installed_observer(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        point: HookPointSpec,
        owning_extension: ironclaw_host_api::ExtensionId,
        scope: HookBindingScope,
        hook: Box<dyn ObserverHook>,
    ) -> Result<(), crate::error::HookError> {
        self.install_observer(
            hook_id,
            phase,
            point,
            HookTrustClass::Installed,
            Some(owning_extension),
            scope,
            hook,
        )
    }

    /// Install an Installed-tier WASM observer hook. Observer failures are
    /// isolated by the failure-policy matrix.
    pub(crate) fn install_installed_wasm_observer(
        &mut self,
        hook_id: HookId,
        phase: HookPhase,
        point: HookPointSpec,
        owning_extension: ironclaw_host_api::ExtensionId,
        scope: HookBindingScope,
        hook: WasmObserverHook,
    ) -> Result<(), crate::error::HookError> {
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase,
            priority: HookPriority::DEFAULT,
            point,
            owning_extension: Some(owning_extension),
            scope,
            poisoned: false,
        };
        self.insert_binding(binding)?;
        self.install_observer_impl(hook_id, ObserverHookImpl::Wasm(hook));
        Ok(())
    }

    /// Dispatch `before_capability`. Hooks run in `(phase, priority, hook_id)`
    /// order. The first `Deny` short-circuits the gate phases; `Telemetry`
    /// phase observers always run.
    pub async fn dispatch_before_capability(
        &self,
        ctx: &BeforeCapabilityHookContext,
    ) -> BeforeCapabilityDispatchOutcome {
        let (ordered, mut poisoned) =
            self.ordered_bindings_with_poison_snapshot(HookPointSpec::BeforeCapability);
        let mut composed = BeforeCapabilityHookDecision::allow();
        let observer_facts = Vec::new();
        let mut failures = Vec::new();
        let mut short_circuited = false;

        for (key, binding) in ordered {
            if short_circuited && !matches!(key.phase, crate::ordering::HookPhase::Telemetry) {
                continue;
            }
            // O(1) poison check against the per-dispatch snapshot. Mid-
            // dispatch poisoning (a hook poisoned by `poison_with_failure`
            // below) inserts into this local set, so the next iteration
            // observes it without re-locking the registry.
            if poisoned.contains(&binding.hook_id) {
                continue;
            }
            // Scope filtering (audit finding C3). The binding's manifest-
            // declared scope is converted to `HookBindingScope` at install
            // time; here we just compare against the resolved capability
            // provider. A hook that doesn't permit the current provider is
            // inert for this invocation — no sink call, no failure, no
            // poisoning. `OwnCapabilities` with an unresolved provider does
            // not fire (conservative default; see `HookBindingScope::permits`).
            if !binding
                .scope
                .permits(binding.owning_extension.as_ref(), ctx.provider.as_ref())
            {
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
                )
                .await;
                poisoned.insert(binding.hook_id);
                if !short_circuited {
                    composed = BeforeCapabilityHookDecision::deny(SanitizedReason::from_static(
                        "hook binding missing implementation",
                    ));
                    short_circuited = true;
                }
                continue;
            };

            self.emit_dispatched(&binding).await;
            let result = self.run_before_capability_hook(hook, &binding, ctx).await;
            match result {
                Ok(GateHookOutcome::Pass) => {
                    // Hook explicitly declared no opinion — contributes
                    // nothing to the composed decision.
                    self.emit_decision(&binding, HookDecisionSummary::Pass)
                        .await;
                }
                Ok(GateHookOutcome::Decision {
                    decision,
                    audit_reason,
                }) => {
                    let summary = telemetry::gate_decision_summary(&decision);
                    self.emit_decision_with_audit(&binding, summary, audit_reason)
                        .await;
                    composed = compose_gate_decision(composed, decision);
                    if !matches!(composed.inner(), GateDecisionInner::Allow) {
                        short_circuited = true;
                    }
                }
                Err(failure) => {
                    self.emit_failure(&failure).await;
                    poisoned.insert(failure.hook_id);
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

        // NOTE: `AfterCapability` observers are NOT drained here. They fire
        // *after* the capability actually executes, which is a different
        // moment than the end of `before_capability` dispatch. The middleware
        // (`HookedLoopCapabilityPort`) is responsible for invoking
        // `dispatch_observer_at(AfterCapability, ...)` once the inner port
        // returns. Observers that need to run *before* the capability
        // invocation should register at `BeforeCapability` phase
        // `Telemetry`.
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
        let (ordered, mut poisoned) =
            self.ordered_bindings_with_poison_snapshot(HookPointSpec::BeforePrompt);
        let mut patches = Vec::new();
        let mut failures = Vec::new();

        for (_key, binding) in ordered {
            if poisoned.contains(&binding.hook_id) {
                continue;
            }
            let Some(hook) = self.before_prompt.get(&binding.hook_id) else {
                self.poison_with_failure(
                    binding.hook_id,
                    FailureCategory::Malformed,
                    binding.trust_class,
                    &crate::trust::DecisionKind::Mutator,
                    "binding present without installed implementation",
                    &mut failures,
                )
                .await;
                poisoned.insert(binding.hook_id);
                continue;
            };
            self.emit_dispatched(&binding).await;
            match self.run_before_prompt_hook(hook, &binding, ctx).await {
                Ok(mut emitted) => {
                    let summary = if emitted.is_empty() {
                        HookDecisionSummary::Pass
                    } else {
                        HookDecisionSummary::Patch
                    };
                    self.emit_decision(&binding, summary).await;
                    patches.append(&mut emitted);
                }
                Err(failure) => {
                    self.emit_failure(&failure).await;
                    poisoned.insert(failure.hook_id);
                    failures.push(failure);
                }
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
        self.dispatch_observer_at_with_provider(point, tenant, None)
            .await
    }

    /// As [`Self::dispatch_observer_at`], but carries the capability provider
    /// for scope-filter enforcement at [`HookPointSpec::AfterCapability`].
    /// Other observer points pass `None`; the dispatcher rejects
    /// `OwnCapabilities`-scoped bindings at non-capability points in the
    /// registry, so this is just defense in depth there.
    pub async fn dispatch_observer_at_with_provider(
        &self,
        point: HookPointSpec,
        tenant: ironclaw_host_api::TenantId,
        provider: Option<ironclaw_host_api::ExtensionId>,
    ) -> ObserverDispatchOutcome {
        let (ordered, mut poisoned) = self.ordered_bindings_with_poison_snapshot(point);
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
                    // Non-observer point passed in: a bug in the dispatcher's
                    // own caller (we should never reach this arm from
                    // production paths). Log so the bug is visible without
                    // crashing the loop.
                    tracing::error!(
                        ?point,
                        "dispatch_observer_at called with non-observer point; \
                         returning empty outcome (this indicates a dispatcher \
                         wiring bug)"
                    );
                    return ObserverDispatchOutcome { facts, failures };
                }
            },
            provider: provider.clone(),
        };

        for (_key, binding) in ordered {
            if poisoned.contains(&binding.hook_id) {
                continue;
            }
            // Scope filtering (serrrfirat finding #3). `OwnCapabilities` is
            // legal at `AfterCapability` because that dispatch carries a
            // resolved provider; for `AfterModel` and `AfterCheckpoint` the
            // registry already rejects `OwnCapabilities` at install time
            // (finding #2), so `provider` is `None` and the scope check would
            // refuse to fire — but the only way to get here at those points
            // is `Global`/`SameTenant`, which `permits` always allows.
            if !binding
                .scope
                .permits(binding.owning_extension.as_ref(), provider.as_ref())
            {
                continue;
            }
            let Some(hook) = self.observers.get(&binding.hook_id) else {
                self.poison_with_failure(
                    binding.hook_id,
                    FailureCategory::Malformed,
                    binding.trust_class,
                    &crate::trust::DecisionKind::Observer,
                    "binding present without installed implementation",
                    &mut failures,
                )
                .await;
                poisoned.insert(binding.hook_id);
                continue;
            };
            self.emit_dispatched(&binding).await;
            match self.run_observer_hook(hook, &binding, &ctx).await {
                Ok(mut emitted) => {
                    self.emit_decision(&binding, HookDecisionSummary::Pass)
                        .await;
                    facts.append(&mut emitted);
                }
                Err(failure) => {
                    self.emit_failure(&failure).await;
                    poisoned.insert(failure.hook_id);
                    failures.push(failure);
                }
            }
        }

        ObserverDispatchOutcome { facts, failures }
    }

    /// As [`Self::ordered_bindings`], but also returns an O(1)-lookup
    /// `HashSet<HookId>` snapshotting the currently-poisoned bindings. The
    /// dispatch loops use this set instead of repeatedly calling
    /// [`Self::is_poisoned`] (which re-locks the registry and walks every
    /// binding) — restoring O(H) dispatch behaviour instead of O(H^2).
    ///
    /// Mid-dispatch poisoning is handled by the caller: when
    /// `poison_with_failure` is called inside the loop, the loop also adds
    /// the hook id to a local copy of this set so subsequent iterations
    /// observe the mid-dispatch poison without a fresh lock.
    fn ordered_bindings_with_poison_snapshot(
        &self,
        point: HookPointSpec,
    ) -> (Vec<(HookOrderKey, HookBinding)>, HashSet<HookId>) {
        let registry = self.registry.lock().expect("hooks registry mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
        let mut out: Vec<_> = registry
            .active_at(point)
            .cloned()
            .map(|b| (HookOrderKey::new(b.phase, b.priority, b.hook_id), b))
            .collect();
        out.sort_by_key(|(k, _)| *k);
        // Walk *all* registry slots once to capture the poison set; this is
        // O(total bindings) under a single lock, then dispatch becomes O(H)
        // HashSet probes.
        let poisoned: HashSet<HookId> = registry.poisoned_ids().collect();
        (out, poisoned)
    }

    #[cfg(test)]
    fn ordered_bindings(&self, point: HookPointSpec) -> Vec<(HookOrderKey, HookBinding)> {
        let registry = self.registry.lock().expect("hooks registry mutex poisoned"); // safety: mutex poison means another thread panicked; failing closed here is correct
        let mut out: Vec<_> = registry
            .active_at(point)
            .cloned()
            .map(|b| {
                let key = HookOrderKey::new(b.phase, b.priority, b.hook_id);
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
        if let BeforeCapabilityHookImpl::RestrictedWasm(h) = hook {
            let run = AssertUnwindSafe(async { h.evaluate(ctx) }).catch_unwind();
            return match tokio::time::timeout(timeout, run).await {
                Ok(Ok(Ok(outcome))) => Ok(outcome),
                Ok(Ok(Err(failure))) => Err(self.classify_wasm_failure(binding, failure)),
                Ok(Err(_panic)) => {
                    Err(self.classify_failure(binding, FailureCategory::Panic, "hook panicked"))
                }
                Err(_elapsed) => Err(self.classify_failure(
                    binding,
                    FailureCategory::Timeout,
                    "hook exceeded dispatch timeout",
                )),
            };
        }

        let run = async {
            match hook {
                BeforeCapabilityHookImpl::Privileged(h) => {
                    let mut sink = RecordingGateSink::new();
                    AssertUnwindSafe(h.evaluate(ctx, &mut sink))
                        .catch_unwind()
                        .await
                        .map_err(|_| ())
                        .map(|()| (sink.state, sink.audit_reason))
                }
                BeforeCapabilityHookImpl::Restricted(h) => {
                    let mut sink = RecordingGateSink::new();
                    AssertUnwindSafe(h.evaluate(ctx, &mut sink))
                        .catch_unwind()
                        .await
                        .map_err(|_| ())
                        .map(|()| (sink.state, sink.audit_reason))
                }
                BeforeCapabilityHookImpl::RestrictedWasm(_) => {
                    // Unreachable: the `RestrictedWasm` variant short-circuits
                    // earlier in this function via a dedicated path that
                    // handles wall-clock timeout + panic isolation around the
                    // WASM invocation. This arm exists only because Rust
                    // exhaustiveness requires it.
                    unreachable!(
                        "RestrictedWasm is dispatched on its own path above; \
                         this branch is unreachable"
                    )
                }
            }
        };

        match tokio::time::timeout(timeout, run).await {
            Ok(Ok((GateSinkState::Decided(decision), audit_reason))) => {
                Ok(GateHookOutcome::Decision {
                    decision,
                    audit_reason,
                })
            }
            Ok(Ok((GateSinkState::Passed, _))) => Ok(GateHookOutcome::Pass),
            Ok(Ok((GateSinkState::Unset, _))) => {
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
        if let BeforePromptHookImpl::RestrictedWasm(h) = hook {
            let run = AssertUnwindSafe(async { h.evaluate(ctx) }).catch_unwind();
            return match tokio::time::timeout(timeout, run).await {
                Ok(Ok(Ok(patches))) => Ok(patches),
                Ok(Ok(Err(failure))) => Err(self.classify_wasm_failure(binding, failure)),
                Ok(Err(_panic)) => {
                    Err(self.classify_failure(binding, FailureCategory::Panic, "hook panicked"))
                }
                Err(_elapsed) => Err(self.classify_failure(
                    binding,
                    FailureCategory::Timeout,
                    "hook exceeded dispatch timeout",
                )),
            };
        }

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
                BeforePromptHookImpl::RestrictedWasm(_) => {
                    // henrypark133 must-fix #2 + #3 on PR #3634: same as
                    // the gate dispatch above — WASM prompt hooks are
                    // dispatched via the early-return guard. The previous
                    // dead arm here also silently discarded the
                    // `WasmHookFailure` category via `|_| ()`, which made
                    // the must-fix #2 silent-failure problem worse on the
                    // prompt path specifically.
                    unreachable!(
                        "wasm prompt hooks dispatched via early-return guard; \
                         this arm is unreachable"
                    )
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
        if let ObserverHookImpl::Wasm(h) = hook {
            let run = AssertUnwindSafe(async { h.observe(ctx) }).catch_unwind();
            return match tokio::time::timeout(timeout, run).await {
                Ok(Ok(Ok(facts))) => Ok(facts),
                Ok(Ok(Err(failure))) => Err(self.classify_wasm_failure(binding, failure)),
                Ok(Err(_panic)) => Err(self.classify_failure(
                    binding,
                    FailureCategory::Panic,
                    "observer hook panicked",
                )),
                Err(_elapsed) => Err(self.classify_failure(
                    binding,
                    FailureCategory::Timeout,
                    "observer hook exceeded dispatch timeout",
                )),
            };
        }

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
                ObserverHookImpl::Wasm(_) => {
                    // henrypark133 must-fix #2 on PR #3634: WASM observer
                    // hooks are dispatched via the early-return guard
                    // above. This arm is unreachable; previously it ran
                    // without `catch_unwind` or timeout and silently
                    // dropped `WasmHookFailure` via `|_| ()`.
                    unreachable!(
                        "wasm observer hooks dispatched via early-return guard; \
                         this arm is unreachable"
                    )
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

    fn classify_wasm_failure(
        &self,
        binding: &HookBinding,
        failure: WasmHookFailure,
    ) -> HookFailureRecord {
        self.classify_failure(binding, failure.category, failure.reason)
    }

    async fn poison_with_failure(
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
        let record = HookFailureRecord {
            hook_id,
            category,
            disposition,
            reason: SanitizedReason::from_static(reason),
        };
        self.emit_failure(&record).await;
        failures.push(record);
    }

    async fn emit_dispatched(&self, binding: &HookBinding) {
        if self.milestone_sink.is_none() {
            return;
        }
        self.emit_milestone(LoopHostMilestoneKind::HookDispatched {
            hook_id: telemetry::hook_id_string(binding.hook_id),
            point: telemetry::point_label(binding.point).to_string(),
            trust_class: telemetry::trust_class_label(binding.trust_class).to_string(),
        })
        .await;
    }

    async fn emit_decision(&self, binding: &HookBinding, decision: HookDecisionSummary) {
        self.emit_decision_with_audit(binding, decision, None).await;
    }

    async fn emit_decision_with_audit(
        &self,
        binding: &HookBinding,
        decision: HookDecisionSummary,
        audit_reason: Option<String>,
    ) {
        if self.milestone_sink.is_none() {
            return;
        }
        self.emit_milestone(LoopHostMilestoneKind::HookDecisionEmitted {
            hook_id: telemetry::hook_id_string(binding.hook_id),
            decision,
            audit_reason: telemetry::sanitize_audit_reason(audit_reason),
        })
        .await;
    }

    async fn emit_failure(&self, record: &HookFailureRecord) {
        if self.milestone_sink.is_none() {
            return;
        }
        self.emit_milestone(LoopHostMilestoneKind::HookFailed {
            hook_id: telemetry::hook_id_string(record.hook_id),
            category: telemetry::failure_category_label(record.category).to_string(),
            disposition: telemetry::failure_disposition_label(record.disposition).to_string(),
        })
        .await;
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

/// Type-enforced builder for [`HookDispatcher`].
///
/// The dispatcher is wired in a specific order: registry → optional timeout
/// → optional milestone sink → installed hooks. After construction it is
/// almost always wrapped in [`Arc`] and handed to a host factory, at which
/// point further mutation is impossible. The builder makes that lifecycle
/// the *only* public construction path: callers chain configuration calls
/// and terminate with [`HookDispatcherBuilder::build_arc`], which performs
/// the `Arc` wrap and returns an immutable handle.
///
/// # Composition order
///
/// ```ignore
/// use std::sync::Arc;
/// use std::time::Duration;
/// use ironclaw_hooks::dispatch::HookDispatcherBuilder;
/// use ironclaw_hooks::registry::HookRegistry;
///
/// let dispatcher = HookDispatcherBuilder::new(HookRegistry::new())
///     .with_timeout(Duration::from_millis(50))
///     // .with_milestone_sink(sink)
///     // .install_builtin_before_capability(...)?
///     .build_arc();
/// # let _ = dispatcher;
/// ```
///
/// Wiring the milestone sink *before* `.build_arc()` is now type-enforced:
/// the sink-attachment method lives on the builder, not on the
/// already-shared `Arc<HookDispatcher>`. Forgetting the sink, or attaching
/// it after Arc-wrapping, becomes a compile-time error rather than a
/// silently-missed configuration step.
#[must_use = "HookDispatcherBuilder does nothing until `.build_arc()` is called"]
pub struct HookDispatcherBuilder {
    dispatcher: HookDispatcher,
}

impl std::fmt::Debug for HookDispatcherBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookDispatcherBuilder")
            .finish_non_exhaustive()
    }
}

impl HookDispatcherBuilder {
    /// Start a new builder from a [`HookRegistry`].
    pub fn new(registry: HookRegistry) -> Self {
        Self {
            dispatcher: HookDispatcher::new(registry),
        }
    }

    /// Override the per-hook wall-clock timeout. Defaults to
    /// [`DEFAULT_HOOK_TIMEOUT`] when not set.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.dispatcher = self.dispatcher.with_timeout(timeout);
        self
    }

    /// Attach a [`HookMilestoneSink`]. See
    /// [`HookDispatcher::with_milestone_sink`] (private) for the contract;
    /// the key benefit of routing this through the builder is that the sink
    /// is wired *before* the dispatcher is shared behind an `Arc`, which is
    /// the only safe time to do so.
    pub fn with_milestone_sink(mut self, sink: Arc<dyn HookMilestoneSink>) -> Self {
        self.dispatcher = self.dispatcher.with_milestone_sink(sink);
        self
    }

    /// Insert a free-standing binding (e.g., from a registrar that has its
    /// own impl-installation flow). Most callers should use one of the
    /// `install_*` helpers below instead.
    pub fn insert_binding(mut self, binding: HookBinding) -> Result<Self, crate::error::HookError> {
        self.dispatcher.insert_binding(binding)?;
        Ok(self)
    }

    // ── Tier-specific public installers, mirroring HookDispatcher ────────

    pub fn install_builtin_before_capability(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        hook: Box<dyn PrivilegedBeforeCapabilityHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher
            .install_builtin_before_capability(hook_id, phase, hook)?;
        Ok(self)
    }

    pub fn install_trusted_before_capability(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        hook: Box<dyn PrivilegedBeforeCapabilityHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher
            .install_trusted_before_capability(hook_id, phase, hook)?;
        Ok(self)
    }

    pub fn install_installed_before_capability(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        owning_extension: ironclaw_host_api::ExtensionId,
        scope: HookBindingScope,
        hook: Box<dyn RestrictedBeforeCapabilityHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher.install_installed_before_capability(
            hook_id,
            phase,
            owning_extension,
            scope,
            hook,
        )?;
        Ok(self)
    }

    pub fn install_builtin_before_prompt(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        hook: Box<dyn PrivilegedBeforePromptHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher
            .install_builtin_before_prompt(hook_id, phase, hook)?;
        Ok(self)
    }

    pub fn install_trusted_before_prompt(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        hook: Box<dyn PrivilegedBeforePromptHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher
            .install_trusted_before_prompt(hook_id, phase, hook)?;
        Ok(self)
    }

    pub fn install_installed_before_prompt(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        owning_extension: ironclaw_host_api::ExtensionId,
        scope: HookBindingScope,
        hook: Box<dyn RestrictedBeforePromptHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher.install_installed_before_prompt(
            hook_id,
            phase,
            owning_extension,
            scope,
            hook,
        )?;
        Ok(self)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn install_observer(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        point: HookPointSpec,
        trust_class: HookTrustClass,
        owning_extension: Option<ironclaw_host_api::ExtensionId>,
        scope: HookBindingScope,
        hook: Box<dyn ObserverHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher.install_observer(
            hook_id,
            phase,
            point,
            trust_class,
            owning_extension,
            scope,
            hook,
        )?;
        Ok(self)
    }

    pub fn install_builtin_observer(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        point: HookPointSpec,
        hook: Box<dyn ObserverHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher
            .install_builtin_observer(hook_id, phase, point, hook)?;
        Ok(self)
    }

    pub fn install_trusted_observer(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        point: HookPointSpec,
        hook: Box<dyn ObserverHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher
            .install_trusted_observer(hook_id, phase, point, hook)?;
        Ok(self)
    }

    pub fn install_installed_observer(
        mut self,
        hook_id: HookId,
        phase: HookPhase,
        point: HookPointSpec,
        owning_extension: ironclaw_host_api::ExtensionId,
        scope: HookBindingScope,
        hook: Box<dyn ObserverHook>,
    ) -> Result<Self, crate::error::HookError> {
        self.dispatcher.install_installed_observer(
            hook_id,
            phase,
            point,
            owning_extension,
            scope,
            hook,
        )?;
        Ok(self)
    }

    /// Get a mutable handle to the still-private dispatcher. Used by the
    /// [`crate::registrar::HookRegistrar`] to install manifest entries
    /// against an in-flight builder without exposing the underlying
    /// installer surface.
    pub(crate) fn dispatcher_mut(&mut self) -> &mut HookDispatcher {
        &mut self.dispatcher
    }

    /// Finalize: wrap the configured dispatcher in [`Arc`]. After this call
    /// the dispatcher can no longer be mutated.
    pub fn build_arc(self) -> Arc<HookDispatcher> {
        Arc::new(self.dispatcher)
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

    fn host_ext() -> ironclaw_host_api::ExtensionId {
        ironclaw_host_api::ExtensionId::new("ext").expect("ext id ok")
    }

    fn ext_hook_id(local: &str) -> HookId {
        HookId::derive(
            &ExtensionId::new("ext").expect("valid ExtensionId in test"),
            "1.0",
            &HookLocalId::new(local).expect("valid HookLocalId in test"),
            HookVersion::ONE,
        )
    }

    fn installed_binding(id: HookId, point: HookPointSpec, phase: HookPhase) -> HookBinding {
        installed_binding_with_priority(id, point, phase, HookPriority::DEFAULT)
    }

    fn installed_binding_with_priority(
        id: HookId,
        point: HookPointSpec,
        phase: HookPhase,
        priority: HookPriority,
    ) -> HookBinding {
        HookBinding {
            hook_id: id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase,
            priority,
            point,
            owning_extension: None,
            scope: HookBindingScope::Global,
            poisoned: false,
        }
    }

    fn ctx() -> BeforeCapabilityHookContext {
        BeforeCapabilityHookContext::new_unresolved(tenant(), "cap.x".to_string(), [0u8; 32])
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

    /// Documents the load-bearing invariant introduced by the builder: from
    /// outside the crate, the only way to obtain a `HookDispatcher` is via
    /// `HookDispatcherBuilder::new(...).build_arc()`. `HookDispatcher::new`,
    /// `.with_timeout`, `.with_milestone_sink`, and every `install_*` method
    /// on the dispatcher are now `pub(crate)` — the type system enforces
    /// the milestone-sink-before-Arc wiring order rather than relying on a
    /// documentation convention.
    ///
    /// This is a compile-fact test, not a runtime assertion. The proof is
    /// the visibility modifier on each method (verified by attempting to
    /// call them from any downstream crate — which would fail to compile)
    /// plus the `Arc` return type of `build_arc`, which forbids further
    /// mutation.
    #[test]
    fn builder_build_arc_is_the_only_public_construction_path() {
        // Sanity: the builder is publicly constructible and produces an
        // Arc<HookDispatcher>. Once handed back as Arc, the dispatcher
        // cannot be mutated (no &mut access through Arc, and the inherent
        // mutators are pub(crate) anyway).
        let dispatcher: Arc<HookDispatcher> =
            HookDispatcherBuilder::new(HookRegistry::new()).build_arc();
        let _ = dispatcher;

        // The following lines, if uncommented from an external crate, would
        // fail to compile:
        //
        //     HookDispatcher::new(HookRegistry::new());
        //     dispatcher.with_timeout(Duration::from_millis(10));
        //     dispatcher.with_milestone_sink(sink);
        //     dispatcher.install_builtin_before_capability(...);
        //
        // (See visibility annotations on each method.)
        let _seal_documented = true;
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
            priority: HookPriority::DEFAULT,
            point: HookPointSpec::BeforeCapability,
            owning_extension: None,
            scope: HookBindingScope::Global,
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
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::BeforePrompt,
                owning_extension: None,
                scope: HookBindingScope::Global,
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
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::AfterModel,
                owning_extension: None,
                scope: HookBindingScope::Global,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_observer_impl(id, ObserverHookImpl::Any(Box::new(NotingObserver)));

        let outcome = dispatcher
            .dispatch_observer_at(HookPointSpec::AfterModel, tenant())
            .await;
        assert_eq!(outcome.facts.len(), 1);
        assert!(outcome.failures.is_empty());
    }

    /// serrrfirat finding #3: an Installed observer scoped to
    /// `OwnCapabilities` must fire only when the dispatch's resolved
    /// capability provider equals the binding's owning extension. The
    /// pre-fix dispatcher fired the observer for every invocation
    /// regardless of provider.
    #[tokio::test]
    async fn own_capabilities_observer_filters_foreign_providers() {
        let owner = ironclaw_host_api::ExtensionId::new("ext.owner").expect("ok");
        let other = ironclaw_host_api::ExtensionId::new("ext.other").expect("ok");
        let id = HookId::derive(
            &crate::identity::ExtensionId::new("ext.owner").expect("valid ExtensionId in test"),
            "1.0",
            &crate::identity::HookLocalId::new("obs").expect("valid HookLocalId in test"),
            HookVersion::ONE,
        );
        let mut registry = HookRegistry::new();
        registry
            .insert(HookBinding {
                hook_id: id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Installed,
                phase: HookPhase::Telemetry,
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::AfterCapability,
                owning_extension: Some(owner.clone()),
                scope: HookBindingScope::OwnCapabilities,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_observer_impl(id, ObserverHookImpl::Any(Box::new(NotingObserver)));

        // Foreign provider — observer must NOT fire.
        let outcome = dispatcher
            .dispatch_observer_at_with_provider(
                HookPointSpec::AfterCapability,
                tenant(),
                Some(other.clone()),
            )
            .await;
        assert!(
            outcome.facts.is_empty(),
            "OwnCapabilities observer fired for foreign provider"
        );

        // Matching provider — observer fires.
        let outcome = dispatcher
            .dispatch_observer_at_with_provider(
                HookPointSpec::AfterCapability,
                tenant(),
                Some(owner.clone()),
            )
            .await;
        assert_eq!(outcome.facts.len(), 1);

        // Unresolved provider (`None`) — observer must NOT fire (conservative
        // default in `HookBindingScope::permits`).
        let outcome = dispatcher
            .dispatch_observer_at_with_provider(HookPointSpec::AfterCapability, tenant(), None)
            .await;
        assert!(
            outcome.facts.is_empty(),
            "OwnCapabilities observer fired against unresolved provider"
        );
    }

    // ── C1 regression: trust-class × impl-tier pairing is sealed ────────────

    /// Compile-time seal. `BeforeCapabilityHookImpl::Privileged(...)` is
    /// `pub(crate)`. There is no public path to pair an `Installed` binding
    /// with a `Privileged` impl because the variant cannot be constructed
    /// from outside the crate. This test documents the load-bearing fact
    /// rather than asserting on a value — the proof is the visibility
    /// modifier on the enum at the top of this file.
    #[test]
    fn compile_time_seal_test() {
        // The following line, if uncommented from an external crate, would
        // fail to compile:
        //
        //     BeforeCapabilityHookImpl::Privileged(Box::new(my_hook))
        //
        // Reachable only from inside `ironclaw_hooks`. External callers must
        // route through `install_builtin_*` / `install_trusted_*` /
        // `install_installed_*`, each of which constructs the binding with
        // the matching trust class.
        let _seal_documented = true;
    }

    /// Even though we can *internally* construct an Installed binding paired
    /// with a Privileged impl in this test, the C1 fix is that there is no
    /// *public* API that lets a caller do so. The public installers each fix
    /// the trust class to match the impl trait. This test exercises every
    /// public installer to prove the trust class is set correctly.
    #[tokio::test]
    async fn public_installers_set_matching_trust_class() {
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());

        let builtin_id = HookId::for_builtin("c1::builtin", HookVersion::ONE);
        dispatcher
            .install_builtin_before_capability(
                builtin_id,
                HookPhase::Policy,
                Box::new(AllowingBuiltinHook),
            )
            .expect("builtin installs at policy");

        let trusted_id = HookId::for_builtin("c1::trusted", HookVersion::ONE);
        dispatcher
            .install_trusted_before_capability(
                trusted_id,
                HookPhase::Policy,
                Box::new(AllowingBuiltinHook),
            )
            .expect("trusted installs at policy");

        let installed_id = ext_hook_id("c1-installed");
        dispatcher
            .install_installed_before_capability(
                installed_id,
                HookPhase::Policy,
                host_ext(),
                HookBindingScope::Global,
                Box::new(PassingInstalledHook),
            )
            .expect("installed installs at policy");

        let registry = dispatcher.registry.lock().expect("registry");
        let bindings: Vec<_> = registry
            .active_at(HookPointSpec::BeforeCapability)
            .cloned()
            .collect();
        let by_id: std::collections::HashMap<HookId, HookTrustClass> = bindings
            .iter()
            .map(|b| (b.hook_id, b.trust_class))
            .collect();
        assert_eq!(by_id.get(&builtin_id), Some(&HookTrustClass::Builtin));
        assert_eq!(by_id.get(&trusted_id), Some(&HookTrustClass::Trusted));
        assert_eq!(by_id.get(&installed_id), Some(&HookTrustClass::Installed));
    }

    /// The `install_installed_before_capability` installer takes
    /// `Box<dyn RestrictedBeforeCapabilityHook>` and constructs the binding
    /// with `HookTrustClass::Installed`. Its impl trait does not expose
    /// `allow()` on its sink (`RestrictedGateSink` has no `.allow()`). So
    /// even a malicious Installed hook cannot mint `Allow` through this
    /// path — the sink trait is the trust seal.
    #[tokio::test]
    async fn installed_binding_cannot_be_paired_with_privileged_impl() {
        // We cannot construct an "Installed binding + Privileged impl" pair
        // through the public API at all; trying to install a privileged hook
        // via `install_installed_before_capability` is a type error. The
        // best we can do at runtime is prove that the installer accepts only
        // Restricted impls and that the resulting sink cannot allow.
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let id = ext_hook_id("c1-restricted-only");
        dispatcher
            .install_installed_before_capability(
                id,
                HookPhase::Policy,
                host_ext(),
                HookBindingScope::Global,
                Box::new(DenyingInstalledHook),
            )
            .expect("installed installs at policy");

        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(
            !outcome.decision.permits(),
            "Installed-tier deny must not be overridable through this path"
        );
    }

    // ── Loader contract regression guard ────────────────────────────────────

    /// The tier-specific installers
    /// (`install_builtin_*` / `install_trusted_*` / `install_installed_*`)
    /// are the *only* public path through which a hook implementation enters
    /// the dispatcher. The `BeforeCapabilityHookImpl::{Privileged, Restricted}`
    /// variants are sealed `pub(crate)`, so no caller outside this crate can
    /// pair a wrong-tier impl with a binding.
    ///
    /// What the type system **does not** enforce is *origin*: if a loader in
    /// `ironclaw_reborn` reads a registry-sourced extension hook and
    /// accidentally routes it through `install_builtin_before_capability`,
    /// the dispatcher will install it as a Builtin. That trust-class ↔ source
    /// pairing is the loader's contractual responsibility — see the
    /// "Loader responsibility" section in `crates/ironclaw_hooks/CLAUDE.md`.
    ///
    /// This test is a regression guard, not a runtime check: it touches each
    /// public tier-specific installer for both `before_capability` and
    /// `before_prompt` so that *any* change to those signatures (rename,
    /// new parameter, removed method) forces this test — and the loader
    /// contract attached to it — to be re-evaluated.
    #[tokio::test]
    async fn tier_specific_installers_are_documented_as_loader_contract() {
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());

        // ── before_capability: all three tier-specific installers ──────────
        let builtin_cap = HookId::for_builtin("loader::builtin::cap", HookVersion::ONE);
        dispatcher
            .install_builtin_before_capability(
                builtin_cap,
                HookPhase::Policy,
                Box::new(AllowingBuiltinHook),
            )
            .expect("install_builtin_before_capability signature stable");

        let trusted_cap = HookId::for_builtin("loader::trusted::cap", HookVersion::ONE);
        dispatcher
            .install_trusted_before_capability(
                trusted_cap,
                HookPhase::Policy,
                Box::new(AllowingBuiltinHook),
            )
            .expect("install_trusted_before_capability signature stable");

        let installed_cap = ext_hook_id("loader-installed-cap");
        dispatcher
            .install_installed_before_capability(
                installed_cap,
                HookPhase::Policy,
                ironclaw_host_api::ExtensionId::new("loader-installed").expect("valid ext"),
                crate::registry::HookBindingScope::Global,
                Box::new(PassingInstalledHook),
            )
            .expect("install_installed_before_capability signature stable");

        // ── before_prompt: all three tier-specific installers ──────────────
        struct NoopPrivilegedPrompt;
        #[async_trait]
        impl crate::sink::PrivilegedBeforePromptHook for NoopPrivilegedPrompt {
            async fn evaluate(
                &self,
                _ctx: &BeforePromptHookContext,
                _sink: &mut dyn crate::sink::PrivilegedMutatorSink,
            ) {
            }
        }

        let builtin_prompt = HookId::for_builtin("loader::builtin::prompt", HookVersion::ONE);
        dispatcher
            .install_builtin_before_prompt(
                builtin_prompt,
                HookPhase::Policy,
                Box::new(NoopPrivilegedPrompt),
            )
            .expect("install_builtin_before_prompt signature stable");

        let trusted_prompt = HookId::for_builtin("loader::trusted::prompt", HookVersion::ONE);
        dispatcher
            .install_trusted_before_prompt(
                trusted_prompt,
                HookPhase::Policy,
                Box::new(NoopPrivilegedPrompt),
            )
            .expect("install_trusted_before_prompt signature stable");

        let installed_prompt = ext_hook_id("loader-installed-prompt");
        dispatcher
            .install_installed_before_prompt(
                installed_prompt,
                HookPhase::Policy,
                ironclaw_host_api::ExtensionId::new("loader-installed").expect("valid ext"),
                crate::registry::HookBindingScope::Global,
                Box::new(EnvelopePatchHook),
            )
            .expect("install_installed_before_prompt signature stable");

        // Verify each binding carries the trust class matching its installer.
        // The loader's responsibility is to pick the installer that matches
        // the *source* of the hook; this test confirms that, given a correct
        // loader choice, the dispatcher records the matching trust class.
        let registry = dispatcher.registry.lock().expect("registry lock");
        let by_id: std::collections::HashMap<HookId, HookTrustClass> = registry
            .active_at(HookPointSpec::BeforeCapability)
            .chain(registry.active_at(HookPointSpec::BeforePrompt))
            .map(|b| (b.hook_id, b.trust_class))
            .collect();
        assert_eq!(by_id.get(&builtin_cap), Some(&HookTrustClass::Builtin));
        assert_eq!(by_id.get(&trusted_cap), Some(&HookTrustClass::Trusted));
        assert_eq!(by_id.get(&installed_cap), Some(&HookTrustClass::Installed));
        assert_eq!(by_id.get(&builtin_prompt), Some(&HookTrustClass::Builtin));
        assert_eq!(by_id.get(&trusted_prompt), Some(&HookTrustClass::Trusted));
        assert_eq!(
            by_id.get(&installed_prompt),
            Some(&HookTrustClass::Installed)
        );
    }

    // ── C5 regression: dedupe + mid-dispatch poison re-check ────────────────

    /// A hook that always panics; used to drive the dispatcher into poisoning
    /// a slot before the snapshot is fully consumed.
    struct AlwaysPanicHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for AlwaysPanicHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            _sink: &mut dyn RestrictedGateSink,
        ) {
            panic!("c5 intentional panic");
        }
    }

    #[tokio::test]
    async fn poisoned_during_dispatch_skips_subsequent_invocations() {
        // First dispatch poisons the slot via a panic.
        let id = ext_hook_id("c5-poisoner");
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        dispatcher
            .install_installed_before_capability(
                id,
                HookPhase::Policy,
                host_ext(),
                HookBindingScope::Global,
                Box::new(AlwaysPanicHook),
            )
            .expect("installs ok");

        let first = dispatcher.dispatch_before_capability(&ctx()).await;
        assert_eq!(first.failures.len(), 1, "first call records the panic");
        assert!(
            dispatcher
                .registry
                .lock()
                .expect("registry")
                .is_poisoned(id),
            "slot must be poisoned after panic"
        );

        // Second dispatch must NOT invoke the panicking hook again — the
        // poison re-check inside the loop has to skip it. If the re-check is
        // missing, the panic would happen a second time and a fresh failure
        // record would appear here.
        let second = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(
            second.failures.is_empty(),
            "poisoned hook must not be re-invoked, got failures: {:?}",
            second.failures
        );
        assert!(
            second.decision.permits(),
            "with no live hooks, composed decision is allow"
        );
    }

    // ─── Milestone telemetry ────────────────────────────────────────────

    use ironclaw_turns::run_profile::{InMemoryHookMilestoneSink, LoopHostMilestoneKind};

    fn install_milestone_sink(
        dispatcher: HookDispatcher,
    ) -> (HookDispatcher, Arc<InMemoryHookMilestoneSink>) {
        let sink = Arc::new(InMemoryHookMilestoneSink::default());
        let dispatcher = dispatcher.with_milestone_sink(Arc::clone(&sink) as Arc<_>);
        (dispatcher, sink)
    }

    #[tokio::test]
    async fn before_capability_emits_dispatched_and_decision_milestones() {
        let id = ext_hook_id("deny-with-tele");
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
        let (dispatcher, sink) = install_milestone_sink(dispatcher);

        let _ = dispatcher.dispatch_before_capability(&ctx()).await;

        let kinds = sink.kinds();
        // Expect: HookDispatched then HookDecisionEmitted(Deny). Trailing
        // AfterCapability observer dispatch has no bindings so no extra
        // milestones are produced.
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, LoopHostMilestoneKind::HookDispatched { .. })),
            "expected HookDispatched milestone, got {kinds:?}"
        );
        let decision_kinds: Vec<_> = kinds
            .iter()
            .filter_map(|k| match k {
                LoopHostMilestoneKind::HookDecisionEmitted { decision, .. } => Some(decision),
                _ => None,
            })
            .collect();
        assert_eq!(decision_kinds.len(), 1, "expected exactly one decision");
        assert_eq!(decision_kinds[0].kind_name(), "deny");
    }

    #[tokio::test]
    async fn before_capability_emits_failed_milestone_on_panic() {
        let id = ext_hook_id("panic-tele");
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
        let (dispatcher, sink) = install_milestone_sink(dispatcher);

        let _ = dispatcher.dispatch_before_capability(&ctx()).await;

        let kinds = sink.kinds();
        let failures: Vec<_> = kinds
            .iter()
            .filter_map(|k| match k {
                LoopHostMilestoneKind::HookFailed {
                    category,
                    disposition,
                    ..
                } => Some((category.as_str(), disposition.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(failures.len(), 1, "expected one failure milestone");
        assert_eq!(failures[0], ("panic", "fail_closed"));
    }

    #[tokio::test]
    async fn before_prompt_emits_dispatched_and_patch_milestones() {
        let id = ext_hook_id("envelope-tele");
        let mut registry = HookRegistry::new();
        registry
            .insert(HookBinding {
                hook_id: id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Installed,
                phase: HookPhase::Policy,
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::BeforePrompt,
                owning_extension: None,
                scope: HookBindingScope::Global,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_prompt(
            id,
            BeforePromptHookImpl::Restricted(Box::new(EnvelopePatchHook)),
        );
        let (dispatcher, sink) = install_milestone_sink(dispatcher);

        let ctx = BeforePromptHookContext::new(tenant(), 4096);
        let _ = dispatcher.dispatch_before_prompt(&ctx).await;

        let kinds = sink.kinds();
        assert_eq!(
            kinds.len(),
            2,
            "expected dispatched + decision, got {kinds:?}"
        );
        assert!(matches!(
            &kinds[0],
            LoopHostMilestoneKind::HookDispatched { point, .. } if point == "before_prompt"
        ));
        assert!(matches!(
            &kinds[1],
            LoopHostMilestoneKind::HookDecisionEmitted { decision, .. }
                if decision.kind_name() == "patch"
        ));
    }

    #[tokio::test]
    async fn observer_dispatch_emits_milestones() {
        let id = HookId::for_builtin("test::observer::tele", HookVersion::ONE);
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        dispatcher
            .install_builtin_observer(
                id,
                HookPhase::Telemetry,
                HookPointSpec::AfterModel,
                Box::new(NotingObserver),
            )
            .expect("install builtin observer");
        let (dispatcher, sink) = install_milestone_sink(dispatcher);

        let _ = dispatcher
            .dispatch_observer_at(HookPointSpec::AfterModel, tenant())
            .await;

        let kinds = sink.kinds();
        assert_eq!(kinds.len(), 2);
        match &kinds[0] {
            LoopHostMilestoneKind::HookDispatched {
                point, trust_class, ..
            } => {
                assert_eq!(point, "after_model");
                assert_eq!(trust_class, "builtin");
            }
            other => panic!("unexpected first milestone: {other:?}"),
        }
        assert!(matches!(
            &kinds[1],
            LoopHostMilestoneKind::HookDecisionEmitted { decision, .. }
                if decision.kind_name() == "pass"
        ));
    }

    /// serrrfirat P2 #2 on PR #3573: installing an observer at a
    /// gate/mutator point used to succeed (binding was inserted, but only
    /// the observer map was populated). Dispatch would later poison the
    /// slot and fail-close the capability. Reject at install time.
    #[test]
    fn install_observer_rejects_before_capability_point() {
        let id = HookId::for_builtin("test::observer::misuse-bc", HookVersion::ONE);
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let err = dispatcher
            .install_builtin_observer(
                id,
                HookPhase::Telemetry,
                HookPointSpec::BeforeCapability,
                Box::new(NotingObserver),
            )
            .expect_err("observer install at before_capability must be rejected");
        match err {
            crate::error::HookError::RegistryConstruction(msg) => {
                assert!(
                    msg.contains("observer") && msg.contains("BeforeCapability"),
                    "unexpected error message: {msg}"
                );
            }
            other => panic!("expected RegistryConstruction, got {other:?}"),
        }
    }

    #[test]
    fn install_observer_rejects_before_prompt_point() {
        let id = HookId::for_builtin("test::observer::misuse-bp", HookVersion::ONE);
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let err = dispatcher
            .install_builtin_observer(
                id,
                HookPhase::Telemetry,
                HookPointSpec::BeforePrompt,
                Box::new(NotingObserver),
            )
            .expect_err("observer install at before_prompt must be rejected");
        assert!(matches!(
            err,
            crate::error::HookError::RegistryConstruction(_)
        ));
    }

    // ─── L4 pairing-invariant matrix ────────────────────────────────────

    /// A hook that emits PauseApproval through the privileged sink. Used to
    /// drive the matrix test through the pause-approval terminator.
    struct PauseApprovalBuiltinHook;
    #[async_trait]
    impl PrivilegedBeforeCapabilityHook for PauseApprovalBuiltinHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn PrivilegedGateSink,
        ) {
            sink.pause_approval("needs human approval");
        }
    }

    /// A hook that emits PauseAuth through the privileged sink.
    struct PauseAuthBuiltinHook;
    #[async_trait]
    impl PrivilegedBeforeCapabilityHook for PauseAuthBuiltinHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn PrivilegedGateSink,
        ) {
            sink.pause_auth("needs re-authentication");
        }
    }

    /// What terminator shape a scenario is expected to produce.
    #[derive(Debug, Clone, Copy)]
    enum ExpectedTerminator {
        /// One `HookDispatched` followed by one `HookDecisionEmitted`.
        Decision,
        /// One `HookDispatched` followed by one `HookFailed`.
        Failure,
        /// A single `HookFailed` with no preceding `HookDispatched`. Used for
        /// the missing-impl scenario, where the dispatcher discovers the
        /// protocol violation *before* the hook is dispatched — the slot is
        /// poisoned without a paired dispatched event. This is documented
        /// here so future changes to the dispatcher's protocol-violation
        /// path don't silently break consumers that depend on the pairing
        /// invariant for *dispatched* hooks.
        FailureWithoutDispatch,
    }

    /// Assert that the milestone sink recorded the expected pairing shape.
    /// Checks shape only, not exact field values.
    fn assert_milestone_sequence(
        kinds: &[LoopHostMilestoneKind],
        scenario: &str,
        expected: ExpectedTerminator,
    ) {
        match expected {
            ExpectedTerminator::Decision | ExpectedTerminator::Failure => {
                assert_eq!(
                    kinds.len(),
                    2,
                    "[{scenario}] expected exactly 2 milestones (HookDispatched + terminator), got {kinds:?}"
                );
                assert!(
                    matches!(&kinds[0], LoopHostMilestoneKind::HookDispatched { .. }),
                    "[{scenario}] first milestone must be HookDispatched, got {:?}",
                    kinds[0]
                );
                match expected {
                    ExpectedTerminator::Decision => assert!(
                        matches!(&kinds[1], LoopHostMilestoneKind::HookDecisionEmitted { .. }),
                        "[{scenario}] terminator must be HookDecisionEmitted, got {:?}",
                        kinds[1]
                    ),
                    ExpectedTerminator::Failure => assert!(
                        matches!(&kinds[1], LoopHostMilestoneKind::HookFailed { .. }),
                        "[{scenario}] terminator must be HookFailed, got {:?}",
                        kinds[1]
                    ),
                    ExpectedTerminator::FailureWithoutDispatch => unreachable!(),
                }
            }
            ExpectedTerminator::FailureWithoutDispatch => {
                assert_eq!(
                    kinds.len(),
                    1,
                    "[{scenario}] missing-impl path emits exactly one HookFailed (no paired dispatched event), got {kinds:?}"
                );
                assert!(
                    matches!(&kinds[0], LoopHostMilestoneKind::HookFailed { .. }),
                    "[{scenario}] sole milestone must be HookFailed, got {:?}",
                    kinds[0]
                );
            }
        }
    }

    async fn run_single_hook_scenario_with_sink<F>(
        scenario: &str,
        install: F,
    ) -> Vec<LoopHostMilestoneKind>
    where
        F: FnOnce(&mut HookDispatcher, HookId),
    {
        let id = ext_hook_id(scenario);
        let mut registry = HookRegistry::new();
        // Note: matrix scenarios run via direct `install_before_capability`,
        // so we register the binding here. The "missing impl" scenario reuses
        // this binding-only path (its `install` closure is a no-op).
        registry
            .insert(installed_binding(
                id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
            ))
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry).with_timeout(Duration::from_millis(20));
        install(&mut dispatcher, id);
        let (dispatcher, sink) = install_milestone_sink(dispatcher);
        let _ = dispatcher.dispatch_before_capability(&ctx()).await;
        sink.kinds()
    }

    #[tokio::test]
    async fn milestones_are_paired_for_all_outcomes() {
        // For each scenario, install a hook (or skip install for "missing
        // impl"), dispatch, and assert the milestone sink has exactly one
        // HookDispatched followed by exactly one terminator. Builtin variant
        // is used where the outcome requires `Privileged` sink access
        // (Allow/PauseApproval/PauseAuth), but the matrix is exercising the
        // milestone pairing invariant, not the trust-class taxonomy.

        // 1. Allow (Privileged Builtin path)
        let kinds = run_single_hook_scenario_with_sink("allow-out", |d, id| {
            d.install_before_capability(
                id,
                BeforeCapabilityHookImpl::Privileged(Box::new(AllowingBuiltinHook)),
            );
        })
        .await;
        assert_milestone_sequence(&kinds, "allow", ExpectedTerminator::Decision);

        // 2. Deny
        let kinds = run_single_hook_scenario_with_sink("deny-out", |d, id| {
            d.install_before_capability(
                id,
                BeforeCapabilityHookImpl::Restricted(Box::new(DenyingInstalledHook)),
            );
        })
        .await;
        assert_milestone_sequence(&kinds, "deny", ExpectedTerminator::Decision);

        // 3. PauseApproval
        let kinds = run_single_hook_scenario_with_sink("pause-approval-out", |d, id| {
            d.install_before_capability(
                id,
                BeforeCapabilityHookImpl::Privileged(Box::new(PauseApprovalBuiltinHook)),
            );
        })
        .await;
        assert_milestone_sequence(&kinds, "pause_approval", ExpectedTerminator::Decision);

        // 4. PauseAuth
        let kinds = run_single_hook_scenario_with_sink("pause-auth-out", |d, id| {
            d.install_before_capability(
                id,
                BeforeCapabilityHookImpl::Privileged(Box::new(PauseAuthBuiltinHook)),
            );
        })
        .await;
        assert_milestone_sequence(&kinds, "pause_auth", ExpectedTerminator::Decision);

        // 5. Pass (no-opinion)
        let kinds = run_single_hook_scenario_with_sink("pass-out", |d, id| {
            d.install_before_capability(
                id,
                BeforeCapabilityHookImpl::Restricted(Box::new(PassingInstalledHook)),
            );
        })
        .await;
        assert_milestone_sequence(&kinds, "pass", ExpectedTerminator::Decision);

        // 6. Panic
        let kinds = run_single_hook_scenario_with_sink("panic-out", |d, id| {
            d.install_before_capability(
                id,
                BeforeCapabilityHookImpl::Restricted(Box::new(PanickingHook)),
            );
        })
        .await;
        assert_milestone_sequence(&kinds, "panic", ExpectedTerminator::Failure);

        // 7. Timeout
        let kinds = run_single_hook_scenario_with_sink("timeout-out", |d, id| {
            d.install_before_capability(
                id,
                BeforeCapabilityHookImpl::Restricted(Box::new(SlowHook)),
            );
        })
        .await;
        assert_milestone_sequence(&kinds, "timeout", ExpectedTerminator::Failure);

        // 8. Malformed (silent hook — no sink call at all)
        let kinds = run_single_hook_scenario_with_sink("malformed-out", |d, id| {
            d.install_before_capability(
                id,
                BeforeCapabilityHookImpl::Restricted(Box::new(SilentInstalledHook)),
            );
        })
        .await;
        assert_milestone_sequence(&kinds, "malformed", ExpectedTerminator::Failure);

        // 9. Missing impl (binding present but no installed hook impl)
        let kinds =
            run_single_hook_scenario_with_sink("missing-impl-out", |_d, _id| { /* no-op */ }).await;
        assert_milestone_sequence(
            &kinds,
            "missing_impl",
            ExpectedTerminator::FailureWithoutDispatch,
        );
    }

    /// Regression for Firat's HIGH priority finding: the dispatcher must
    /// respect `HookBinding.priority` in the sort key. Install two
    /// `BeforeCapability` Policy hooks with `HookPriority::FIRST` and
    /// `HookPriority::LAST`; the FIRST hook must dispatch before the LAST
    /// hook regardless of their content-addressed hook ids.
    #[tokio::test]
    async fn priority_overrides_hook_id_tiebreaker_in_dispatch_order() {
        let first_id = ext_hook_id("priority-first");
        let last_id = ext_hook_id("priority-last");

        let mut registry = HookRegistry::new();
        registry
            .insert(installed_binding_with_priority(
                first_id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
                HookPriority::FIRST,
            ))
            .expect("ok");
        registry
            .insert(installed_binding_with_priority(
                last_id,
                HookPointSpec::BeforeCapability,
                HookPhase::Policy,
                HookPriority::LAST,
            ))
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            first_id,
            BeforeCapabilityHookImpl::Privileged(Box::new(AllowingBuiltinHook)),
        );
        dispatcher.install_before_capability(
            last_id,
            BeforeCapabilityHookImpl::Privileged(Box::new(AllowingBuiltinHook)),
        );

        let (dispatcher, sink) = install_milestone_sink(dispatcher);
        let _ = dispatcher.dispatch_before_capability(&ctx()).await;
        let kinds = sink.kinds();

        // Pull out the dispatched-event order; with two hooks both producing
        // Allow there's no short-circuit, so both must appear in priority order.
        let dispatched: Vec<String> = kinds
            .iter()
            .filter_map(|k| match k {
                LoopHostMilestoneKind::HookDispatched { hook_id, .. } => Some(hook_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            dispatched,
            vec![first_id.to_hex(), last_id.to_hex()],
            "FIRST must dispatch before LAST regardless of hook-id tiebreaker"
        );
    }

    #[tokio::test]
    async fn milestones_emit_paired_for_each_hook_in_multi_hook_dispatch() {
        // Install 3 hooks at the same point with mixed outcomes (allow, deny,
        // panic). Each hook must emit its own paired HookDispatched +
        // terminator, and the sequences must be interleaved in deterministic
        // (phase, priority, hook_id) order. Because all three share the same
        // phase (Policy) and the default priority, ordering is by hook_id.
        //
        // hook_id derivation is a blake3 hash of the local id string; we can't
        // predict the exact ordering analytically, so we capture the ordered
        // list from the registry itself and assert the milestone stream
        // matches it.
        let allow_id = ext_hook_id("multi-allow");
        let deny_id = ext_hook_id("multi-deny");
        let panic_id = ext_hook_id("multi-panic");

        let mut registry = HookRegistry::new();
        for id in [allow_id, deny_id, panic_id] {
            registry
                .insert(installed_binding(
                    id,
                    HookPointSpec::BeforeCapability,
                    HookPhase::Policy,
                ))
                .expect("ok");
        }
        let mut dispatcher = HookDispatcher::new(registry);
        // Note: we use Privileged for Allow so the sink can mint allow; this
        // is a test of milestone pairing in a multi-hook dispatch, not a
        // trust-class taxonomy test, so mixed impl tiers are acceptable.
        dispatcher.install_before_capability(
            allow_id,
            BeforeCapabilityHookImpl::Privileged(Box::new(AllowingBuiltinHook)),
        );
        dispatcher.install_before_capability(
            deny_id,
            BeforeCapabilityHookImpl::Restricted(Box::new(DenyingInstalledHook)),
        );
        dispatcher.install_before_capability(
            panic_id,
            BeforeCapabilityHookImpl::Restricted(Box::new(PanickingHook)),
        );

        // Capture the expected order from the dispatcher before sealing it.
        let ordered = dispatcher.ordered_bindings(HookPointSpec::BeforeCapability);
        let expected_order: Vec<HookId> = ordered.iter().map(|(_, b)| b.hook_id).collect();
        assert_eq!(
            expected_order.len(),
            3,
            "expected 3 ordered bindings, got {expected_order:?}"
        );

        let (dispatcher, sink) = install_milestone_sink(dispatcher);
        let _ = dispatcher.dispatch_before_capability(&ctx()).await;
        let kinds = sink.kinds();

        // Each hook contributes exactly 2 events. The dispatch may short-
        // circuit after a deny is composed, but the matrix is constructed so
        // *all three* run (Allow first, then Deny short-circuits, but the
        // Panic hook may still run if it sorts before Deny in hook-id order).
        // We assert the structural invariant: every emitted dispatched event
        // is followed by a terminator for the SAME hook id before the next
        // dispatched event appears. Hooks that were short-circuited away
        // emit no milestones at all (the loop `continue`s before
        // `emit_dispatched`).
        let mut i = 0;
        let mut paired_hook_ids: Vec<String> = Vec::new();
        while i < kinds.len() {
            let dispatched_hook_id = match &kinds[i] {
                LoopHostMilestoneKind::HookDispatched { hook_id, .. } => hook_id.clone(),
                other => panic!(
                    "expected HookDispatched at index {i}, got {other:?}; full stream: {kinds:?}"
                ),
            };
            assert!(
                i + 1 < kinds.len(),
                "dangling HookDispatched at end of stream: {kinds:?}"
            );
            let terminator_hook_id = match &kinds[i + 1] {
                LoopHostMilestoneKind::HookDecisionEmitted { hook_id, .. } => hook_id.clone(),
                LoopHostMilestoneKind::HookFailed { hook_id, .. } => hook_id.clone(),
                other => panic!(
                    "expected terminator at index {}, got {other:?}; full stream: {kinds:?}",
                    i + 1
                ),
            };
            assert_eq!(
                dispatched_hook_id, terminator_hook_id,
                "milestone pair has mismatched hook ids; full stream: {kinds:?}"
            );
            paired_hook_ids.push(dispatched_hook_id);
            i += 2;
        }

        // The order of paired-hook-ids must be a prefix of the deterministic
        // ordering taken from the registry (some trailing hooks may be skipped
        // by short-circuit, but no hook may be invoked out of order).
        let expected_hex: Vec<String> = expected_order
            .iter()
            .map(|h| telemetry::hook_id_string(*h))
            .collect();
        assert!(
            paired_hook_ids.len() <= expected_hex.len(),
            "more paired hooks emitted than registered: paired={paired_hook_ids:?} expected={expected_hex:?}"
        );
        for (idx, paired) in paired_hook_ids.iter().enumerate() {
            assert_eq!(
                paired, &expected_hex[idx],
                "milestone order diverges from deterministic registry order at index {idx}: paired={paired_hook_ids:?} expected={expected_hex:?}"
            );
        }
    }

    #[tokio::test]
    async fn no_sink_emits_no_milestones_and_preserves_behavior() {
        // Sanity: dispatcher without a milestone sink still functions and
        // emits nothing. Tested implicitly by the rest of the suite, but
        // asserted explicitly here for the telemetry contract.
        let id = ext_hook_id("no-tele");
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

        // No `with_milestone_sink` call.
        let outcome = dispatcher.dispatch_before_capability(&ctx()).await;
        assert!(!outcome.decision.permits());
    }

    // ── C3 regression: manifest-declared scope enforced at dispatch time ────

    fn host_ext_named(name: &str) -> ironclaw_host_api::ExtensionId {
        ironclaw_host_api::ExtensionId::new(name).expect("valid ext id")
    }

    fn ctx_with_provider(
        capability: &str,
        provider: Option<ironclaw_host_api::ExtensionId>,
    ) -> BeforeCapabilityHookContext {
        BeforeCapabilityHookContext::new(
            tenant(),
            capability.to_string(),
            [0u8; 32],
            crate::points::SanitizedArguments::unresolved(),
            provider,
        )
    }

    #[tokio::test]
    async fn own_capabilities_scope_filters_out_cross_extension_invocation() {
        // Installed hook authored by ext-A, scoped to OwnCapabilities. The
        // capability under invocation is provided by ext-B; the hook must
        // not fire and the composed decision is allow.
        let id = ext_hook_id("c3-own-a");
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        dispatcher
            .install_installed_before_capability(
                id,
                HookPhase::Policy,
                host_ext_named("ext-a"),
                HookBindingScope::OwnCapabilities,
                Box::new(DenyingInstalledHook),
            )
            .expect("install installed hook");

        let outcome = dispatcher
            .dispatch_before_capability(&ctx_with_provider(
                "cap.foo",
                Some(host_ext_named("ext-b")),
            ))
            .await;
        assert!(
            outcome.decision.permits(),
            "OwnCapabilities-scoped hook from ext-A must not fire on a cap provided by ext-B"
        );
        assert!(
            outcome.failures.is_empty(),
            "scope filtering is inert — no failure recorded"
        );
    }

    #[tokio::test]
    async fn own_capabilities_scope_fires_for_matching_extension() {
        let id = ext_hook_id("c3-own-a-self");
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        dispatcher
            .install_installed_before_capability(
                id,
                HookPhase::Policy,
                host_ext_named("ext-a"),
                HookBindingScope::OwnCapabilities,
                Box::new(DenyingInstalledHook),
            )
            .expect("install installed hook");

        let outcome = dispatcher
            .dispatch_before_capability(&ctx_with_provider(
                "cap.foo",
                Some(host_ext_named("ext-a")),
            ))
            .await;
        assert!(
            !outcome.decision.permits(),
            "OwnCapabilities-scoped hook from ext-A must fire on a cap provided by ext-A"
        );
    }

    #[tokio::test]
    async fn own_capabilities_scope_does_not_fire_when_provider_unresolved() {
        // Conservative default: with no resolver wired in, the provider is
        // None and OwnCapabilities-scoped hooks stay inert. This is the
        // documented behavior (see `HookBindingScope::permits`).
        let id = ext_hook_id("c3-own-unresolved");
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        dispatcher
            .install_installed_before_capability(
                id,
                HookPhase::Policy,
                host_ext_named("ext-a"),
                HookBindingScope::OwnCapabilities,
                Box::new(DenyingInstalledHook),
            )
            .expect("install installed hook");

        let outcome = dispatcher
            .dispatch_before_capability(&ctx_with_provider("cap.foo", None))
            .await;
        assert!(
            outcome.decision.permits(),
            "OwnCapabilities-scoped hook must NOT fire when provider is unresolved"
        );
    }

    #[tokio::test]
    async fn same_tenant_scope_fires_regardless_of_provider() {
        let id = ext_hook_id("c3-same-tenant");
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        dispatcher
            .install_installed_before_capability(
                id,
                HookPhase::Policy,
                host_ext_named("ext-a"),
                HookBindingScope::SameTenant,
                Box::new(DenyingInstalledHook),
            )
            .expect("install installed hook");

        // ext-B provider — must still fire.
        let outcome_b = dispatcher
            .dispatch_before_capability(&ctx_with_provider(
                "cap.foo",
                Some(host_ext_named("ext-b")),
            ))
            .await;
        assert!(
            !outcome_b.decision.permits(),
            "SameTenant hook fires regardless of provider (ext-B)"
        );

        // Unresolved provider — must also fire.
        let outcome_none = dispatcher
            .dispatch_before_capability(&ctx_with_provider("cap.foo", None))
            .await;
        assert!(
            !outcome_none.decision.permits(),
            "SameTenant hook fires even when provider is unresolved"
        );
    }

    /// Builtin hook that always denies — used to verify "did the hook
    /// actually fire?" by inspecting whether the composed decision flipped
    /// away from allow.
    struct DenyingBuiltin;
    #[async_trait]
    impl PrivilegedBeforeCapabilityHook for DenyingBuiltin {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn PrivilegedGateSink,
        ) {
            sink.deny("builtin-fires");
        }
    }

    #[tokio::test]
    async fn builtin_global_scope_always_fires() {
        let id = HookId::for_builtin("test::c3::builtin::deny", HookVersion::ONE);
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        dispatcher
            .install_builtin_before_capability(id, HookPhase::Validation, Box::new(DenyingBuiltin))
            .expect("install builtin deny hook");

        // Foreign provider — must fire.
        let outcome_b = dispatcher
            .dispatch_before_capability(&ctx_with_provider(
                "cap.foo",
                Some(host_ext_named("ext-b")),
            ))
            .await;
        assert!(
            !outcome_b.decision.permits(),
            "Builtin (Global) hook must fire regardless of provider"
        );

        // Unresolved provider — must also fire.
        let outcome_none = dispatcher
            .dispatch_before_capability(&ctx_with_provider("cap.foo", None))
            .await;
        assert!(
            !outcome_none.decision.permits(),
            "Builtin (Global) hook must fire even when provider is unresolved"
        );
    }
}
