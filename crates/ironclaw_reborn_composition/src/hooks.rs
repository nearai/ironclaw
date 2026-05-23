//! Production activation of the hook framework.
//!
//! This module is the single composition seam that flips the (otherwise
//! dormant) hook framework into the live capability-invocation path. It owns:
//!
//! 1. **The feature flag** ([`HooksActivationConfig`]) — default OFF. When
//!    OFF, [`build_hook_dispatcher_builder_factory`] returns `None` and the
//!    runtime composes exactly as it did before hooks existed (zero behavior
//!    change). This is the hard rollout-safety contract for the activation.
//! 2. **The first-party builtin hook set** ([`install_first_party_hooks`]) —
//!    installed regardless of extensions. Today this is a single illustrative
//!    no-op observer ([`NoOpObserverHook`]); the production catalog is TBD and
//!    deliberately not invented here. The deliverable is the activation
//!    machinery, not a hook catalog.
//! 3. **The manifest → registry loader** ([`install_extension_hooks`]) — takes
//!    installed extension packages, projects each declared
//!    [`HookSectionEntryV2`] payload into a typed
//!    [`ironclaw_hooks::manifest::HookManifestEntry`], and installs them
//!    through [`HookRegistrar::install`] at the `Installed` trust tier. Trust
//!    attenuation is enforced by construction: the registrar only ever calls
//!    `install_installed_*`, so an extension hook can never mint `Allow` /
//!    `Gate` / `Mutator` without an explicit per-extension grant.
//! 4. **The per-run dispatcher builder factory** — returned to the runtime to
//!    pass to `RebornLoopDriverHostFactory::with_hook_dispatcher_builder_
//!    factory`. The closure mints a *fresh* [`HookDispatcherBuilder`] per host
//!    build (per run), so slot-poisoning and registry mutations never leak
//!    across runs. Telemetry attribution is per-run because the host factory
//!    attaches the run-scoped milestone sink internally to each fresh builder.
//!
//! ## Per-tenant scoping (multi-tenant isolation contract, #3890)
//!
//! `build_reborn_runtime` is invoked once per identity/owner — one
//! `tenant_id` per call. Everything this module constructs (the
//! [`PredicateEvaluator`] + its state backend, the template registry, the
//! per-run dispatcher closure) is built inside that per-tenant call, so one
//! tenant's hooks and predicate counters can never apply to another. There is
//! no global registry.
//!
//! ## Predicate backend (in-memory for v1)
//!
//! The evaluator uses the in-memory predicate-state backend for now. The
//! backend is swappable ([`PredicateEvaluator::with_state_backend`]) so the
//! durable Postgres/libSQL backends (#3933 + follow-ups) can drop in without
//! touching this module's wiring. See the `warn_in_memory_backend_active_in_
//! production` note emitted at construction time.

use std::sync::Arc;

use ironclaw_extensions::ExtensionRegistry;
use ironclaw_hooks::HookPhase;
use ironclaw_hooks::dispatch::HookDispatcherBuilder;
use ironclaw_hooks::evaluator::PredicateEvaluator;
use ironclaw_hooks::identity::{HookId, HookVersion};
use ironclaw_hooks::manifest::HookManifestEntry;
use ironclaw_hooks::points::ObserverHookContext;
use ironclaw_hooks::predicate_state::{InMemoryPredicateStateBackend, PredicateStateBackend};
use ironclaw_hooks::registrar::HookRegistrar;
use ironclaw_hooks::registry::{HookPointSpec, HookRegistry};
use ironclaw_hooks::sink::{ObserverHook, ObserverSink};

use crate::error::RebornBuildError;

/// Canonical identity path for the first-party no-op observer hook.
const NOOP_OBSERVER_CANONICAL_PATH: &str = "ironclaw_reborn_composition::hooks::NoOpObserverHook";

/// Per-host-build factory closure passed to
/// `RebornLoopDriverHostFactory::with_hook_dispatcher_builder_factory`. The
/// closure is invoked once per `build_text_only_host*` call and must return a
/// fresh [`HookDispatcherBuilder`] (no pre-attached milestone sink — the host
/// factory wires a run-scoped one). `Fn + Send + Sync + 'static`.
pub type HookDispatcherBuilderFactory = Arc<dyn Fn() -> HookDispatcherBuilder + Send + Sync>;

/// Activation configuration for the hook framework.
///
/// **Default OFF.** This is the rollout-safety contract: a default-constructed
/// config (or one built from an unset environment) leaves the dispatcher
/// uncomposed, so the production runtime behaves exactly as it did before
/// hooks existed. The flag is flipped to ON deliberately (canary → on), never
/// by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HooksActivationConfig {
    enabled: bool,
}

impl Default for HooksActivationConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}

/// Environment variable that flips the hook framework on. Absent / empty /
/// any value other than a recognized truthy token ⇒ OFF.
pub const HOOKS_ENABLED_ENV: &str = "HOOKS_ENABLED";

impl HooksActivationConfig {
    /// Explicitly enabled.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Explicitly disabled (the default).
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Resolve the activation flag from the process environment. Fail-safe to
    /// OFF: only the canonical truthy tokens (`1`, `true`, `yes`, `on`,
    /// case-insensitive) enable the framework; everything else — including an
    /// unset variable or an unparseable value — leaves it disabled.
    pub fn from_env() -> Self {
        match std::env::var(HOOKS_ENABLED_ENV) {
            Ok(value) => Self {
                enabled: is_truthy(&value),
            },
            Err(_) => Self::disabled(),
        }
    }

    pub fn is_enabled(self) -> bool {
        self.enabled
    }
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// A first-party, builtin no-op observer hook.
///
/// This is the *only* hook shipped in the first-party set today. It proves the
/// builtin install + dispatch path end to end while making zero
/// driver-visible changes even with the flag ON: observers cannot affect
/// outcomes, and this one records nothing. The production first-party catalog
/// is TBD — no real first-party deny/policy hook has been productized, and
/// this PR deliberately does not invent one.
#[derive(Debug, Default)]
pub struct NoOpObserverHook;

#[async_trait::async_trait]
impl ObserverHook for NoOpObserverHook {
    async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
        // Intentionally empty. The activation deliverable is the install +
        // dispatch path; this hook ships dark.
    }
}

/// Install the first-party builtin hook set into `builder`.
///
/// Builtin hooks are `Builtin`-tier (full authority within the framework) and
/// are identified by a stable canonical path, not a content-addressed
/// extension id. They are installed regardless of which extensions are
/// present.
fn install_first_party_hooks(
    mut builder: HookDispatcherBuilder,
) -> Result<HookDispatcherBuilder, RebornBuildError> {
    let hook_id = HookId::for_builtin(NOOP_OBSERVER_CANONICAL_PATH, HookVersion::ONE);
    builder = builder
        .install_builtin_observer(
            hook_id,
            HookPhase::Telemetry,
            HookPointSpec::AfterCapability,
            Box::new(NoOpObserverHook),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("failed to install first-party no-op observer hook: {error}"),
        })?;
    Ok(builder)
}

/// Project the structurally-typed [`HookSectionEntryV2`] payloads declared by
/// every extension in `registry` into typed
/// [`HookManifestEntry`] values and install them at the `Installed` trust tier
/// through `registrar`.
///
/// This is the manifest → registry loader. It is the *only* place the
/// `ExtensionManifestV2` hook DTO crosses into the hook crate's typed
/// vocabulary, satisfying the clean-boundary contract: `ironclaw_extensions`
/// stays free of hook types, and the projection happens here, in the crate
/// that depends on both.
///
/// Trust attenuation is enforced by construction — the registrar only ever
/// calls `install_installed_*`, which type-level-prevents an extension hook
/// from minting `Allow` / `Gate` / `Mutator` without an explicit grant
/// (`ironclaw_hooks::trust`).
///
/// On any projection or install failure the whole build fails loudly
/// (fail-closed). A malformed manifest hook must never silently drop into a
/// half-installed registry.
fn install_extension_hooks(
    registry: &ExtensionRegistry,
    registrar: &HookRegistrar,
    mut builder: HookDispatcherBuilder,
) -> Result<HookDispatcherBuilder, RebornBuildError> {
    for package in registry.extensions() {
        let manifest = &package.manifest;
        if manifest.hooks.is_empty() {
            continue;
        }
        let extension_id = manifest.id.clone();
        let extension_version = manifest.version.clone();

        let mut entries = Vec::with_capacity(manifest.hooks.len());
        for hook in &manifest.hooks {
            // Re-parse the canonical TOML the v2 parser preserved into the
            // typed hook entry. This is the clean-boundary projection: the
            // extension crate never knew the hook vocabulary; we do.
            let entry: HookManifestEntry = toml::from_str(&hook.raw_toml).map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!(
                        "extension `{}` hook `{}` is not a valid hook manifest entry: {error}",
                        extension_id.as_str(),
                        hook.local_id
                    ),
                }
            })?;
            entries.push(entry);
        }

        let (next, _hook_ids) = registrar
            .install(extension_id.clone(), extension_version, entries, builder)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!(
                    "failed to install hooks declared by extension `{}`: {error}",
                    extension_id.as_str()
                ),
            })?;
        builder = next;
    }
    Ok(builder)
}

/// Build the per-run hook dispatcher builder factory for the production
/// runtime, or `None` when the framework is disabled.
///
/// - **Flag OFF** ⇒ returns `Ok(None)`. The runtime never composes a
///   dispatcher; behavior is identical to the pre-hooks runtime. This is the
///   default and the rollout-safety contract.
/// - **Flag ON** ⇒ projects + installs the first-party builtin hooks and every
///   extension-declared hook into a *template* registry once (fail-closed on
///   any error), then returns a closure that mints a fresh
///   [`HookDispatcherBuilder`] per host build by replaying the same install
///   set. The fresh-per-build construction gives each run its own dispatcher
///   (no cross-run poison/counter leak), and the per-tenant `registry` +
///   evaluator passed in keep one tenant's hooks isolated from another.
///
/// `registry` must be the per-tenant extension registry for the runtime being
/// composed. The returned factory closure captures the install inputs (cloned
/// manifest entries + the shared evaluator), not a built dispatcher, so each
/// invocation produces an independent dispatcher.
pub fn build_hook_dispatcher_builder_factory(
    config: HooksActivationConfig,
    registry: &ExtensionRegistry,
) -> Result<Option<HookDispatcherBuilderFactory>, RebornBuildError> {
    if !config.is_enabled() {
        return Ok(None);
    }

    // In-memory predicate-state backend for v1. Swappable: a durable
    // Postgres/libSQL backend (#3933) drops in here without touching the rest
    // of the wiring.
    let backend: Arc<dyn PredicateStateBackend> = Arc::new(InMemoryPredicateStateBackend::new());
    let evaluator = Arc::new(PredicateEvaluator::with_state_backend(Arc::clone(&backend)));
    evaluator.warn_in_memory_backend_active_in_production();

    // Pre-project + validate every extension hook *once*, fail-closed, so a
    // malformed manifest fails the build rather than surfacing per-run. We
    // build a template dispatcher to prove the full install set is valid, then
    // capture the inputs needed to rebuild it fresh per run.
    let registrar = HookRegistrar::new(Arc::clone(&evaluator));

    // Validate the full install set up front against a scratch builder. If
    // this fails, the build fails loudly (fail-closed). An empty registry
    // (first-party-only) is a legitimate state — the no-op observer alone is
    // a valid composed dispatcher.
    {
        let scratch = HookDispatcherBuilder::new(HookRegistry::new());
        let scratch = install_first_party_hooks(scratch)?;
        let _validated = install_extension_hooks(registry, &registrar, scratch)?;
    }

    // Collect the per-extension typed entries once so the per-run closure
    // doesn't re-parse TOML on every host build. Cloning `HookManifestEntry`
    // is cheap relative to the per-run dispatch cost.
    let mut extension_install_sets: Vec<(
        ironclaw_host_api::ExtensionId,
        String,
        Vec<HookManifestEntry>,
    )> = Vec::new();
    for package in registry.extensions() {
        let manifest = &package.manifest;
        if manifest.hooks.is_empty() {
            continue;
        }
        let mut entries = Vec::with_capacity(manifest.hooks.len());
        for hook in &manifest.hooks {
            // Already validated above; this parse cannot fail, but we surface
            // any error fail-closed rather than unwrapping.
            let entry: HookManifestEntry = toml::from_str(&hook.raw_toml).map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!(
                        "extension `{}` hook `{}` failed re-projection: {error}",
                        manifest.id.as_str(),
                        hook.local_id
                    ),
                }
            })?;
            entries.push(entry);
        }
        extension_install_sets.push((manifest.id.clone(), manifest.version.clone(), entries));
    }

    let evaluator_for_factory = Arc::clone(&evaluator);
    let factory: HookDispatcherBuilderFactory = Arc::new(move || {
        // Fresh registry + builder per run: no cross-run state leak.
        let mut builder = HookDispatcherBuilder::new(HookRegistry::new());
        builder = install_first_party_hooks(builder)
            .expect("first-party hook install validated at composition time");
        let registrar = HookRegistrar::new(Arc::clone(&evaluator_for_factory));
        for (extension_id, version, entries) in &extension_install_sets {
            let (next, _ids) = registrar
                .install(
                    extension_id.clone(),
                    version.clone(),
                    entries.clone(),
                    builder,
                )
                .expect("extension hook install validated at composition time");
            builder = next;
        }
        builder
    });

    Ok(Some(factory))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_disabled() {
        assert!(!HooksActivationConfig::default().is_enabled());
        assert!(!HooksActivationConfig::disabled().is_enabled());
        assert!(HooksActivationConfig::enabled().is_enabled());
    }

    #[test]
    fn truthy_tokens_enable_only_canonical_values() {
        for token in ["1", "true", "TRUE", "Yes", "on", " on "] {
            assert!(is_truthy(token), "{token:?} should be truthy");
        }
        for token in ["0", "false", "", "off", "no", "enabled", "2", "tru"] {
            assert!(!is_truthy(token), "{token:?} should be falsy");
        }
    }

    #[test]
    fn disabled_config_yields_no_factory() {
        let registry = ExtensionRegistry::new();
        let result =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::disabled(), &registry)
                .expect("disabled build never errors");
        assert!(result.is_none(), "flag OFF must compose no dispatcher");
    }

    #[test]
    fn enabled_config_with_empty_registry_yields_first_party_only_factory() {
        // First-party-only is a legitimate composed state: the no-op observer
        // alone is a valid dispatcher.
        let registry = ExtensionRegistry::new();
        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("enabled build with empty registry succeeds")
                .expect("flag ON yields a factory");
        // The factory mints a builder; build it and confirm the first-party
        // hook is present at the AfterCapability point.
        let dispatcher = factory().build_arc();
        let builtin_id = HookId::for_builtin(NOOP_OBSERVER_CANONICAL_PATH, HookVersion::ONE);
        let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::AfterCapability);
        assert!(
            bindings.iter().any(|binding| binding.hook_id == builtin_id),
            "first-party no-op observer must be installed when the flag is ON"
        );
    }

    #[test]
    fn factory_mints_independent_dispatchers_per_call() {
        let registry = ExtensionRegistry::new();
        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("enabled build succeeds")
                .expect("flag ON yields a factory");
        let a = factory().build_arc();
        let b = factory().build_arc();
        assert!(
            !Arc::ptr_eq(&a, &b),
            "each factory call must mint a fresh dispatcher (per-run isolation)"
        );
    }
}
