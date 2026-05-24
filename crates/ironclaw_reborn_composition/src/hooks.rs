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
//!    installed regardless of extensions. The production catalog is
//!    deliberately **EMPTY**: no real first-party builtin hook has been
//!    productized, so we ship none. A production type + install path + behavior
//!    for a hook that does nothing is scaffolding, not a deliverable. The
//!    activation machinery is exercised end-to-end with test-only hooks (see
//!    the `#[cfg(test)]` `NoOpObserverHook` and the test-only first-party
//!    installer seam below), not with a shipped no-op. An empty first-party set
//!    composed with no extension hooks is a legitimate state — the dispatcher
//!    composes with zero bindings, which is valid (not a panic/error).
//! 3. **The manifest → registry loader** ([`project_extension_install_sets`] +
//!    [`install_extension_sets`]) — projects each declared
//!    [`HookSectionEntryV2`] payload into a typed
//!    [`ironclaw_hooks::manifest::HookManifestEntry`] (the only fallible,
//!    external-input step), then installs them through [`HookRegistrar::install`]
//!    at the `Installed` trust tier. Trust attenuation is enforced by
//!    construction: the registrar only ever calls `install_installed_*`, so an
//!    extension hook can never mint `Allow` / `Gate` / `Mutator` without an
//!    explicit per-extension grant.
//! 4. **The per-run dispatcher builder factory** — a [`HookInstallPlan`]
//!    compiled once (parse + validate the full install set, fail-closed),
//!    returned to the runtime to pass to
//!    `RebornLoopDriverHostFactory::with_hook_dispatcher_builder_factory`. The
//!    factory closure calls [`HookInstallPlan::rebuild`] to mint a *fresh*
//!    [`HookDispatcherBuilder`] per host build (per run), so slot-poisoning and
//!    registry mutations never leak across runs. `rebuild` is infallible **by
//!    construction**: a `HookInstallPlan` only exists for an install set that
//!    already composed cleanly, so replaying it from the identical fresh-empty
//!    start cannot fail (no per-run `.expect()`/`unwrap`). Telemetry
//!    attribution is per-run because the host factory attaches the run-scoped
//!    milestone sink internally to each fresh builder.
//!
//! ## Per-tenant scoping (multi-tenant isolation contract, #3890)
//!
//! `build_reborn_runtime` is invoked once per identity/owner — one
//! `tenant_id` per call. Everything this module constructs (the
//! [`PredicateEvaluator`] + its state backend, the compiled [`HookInstallPlan`],
//! the per-run dispatcher closure) is built inside that per-tenant call, so one
//! tenant's hooks can never apply to another. There is no global registry.
//!
//! ## Predicate counter scoping (TENANT-scoped, deliberately shared across runs)
//!
//! The [`PredicateEvaluator`] and its state backend are constructed **once per
//! runtime/tenant** ([`build_hook_dispatcher_builder_factory_with`]) and the
//! same `Arc<PredicateEvaluator>` is captured into the [`HookInstallPlan`], so
//! every per-run dispatcher minted by [`HookInstallPlan::rebuild`] shares it.
//! This is intentional, not an isolation leak:
//!
//! - Rate-limit and value-cap predicate counters are keyed by
//!   `(hook, tenant, capability)` with **no `run_id`** — they are *definitionally*
//!   tenant-scoped. A run-scoped rate limit would reset to zero at the start of
//!   every run and could never enforce a cross-run budget, which is useless.
//! - So the counter STATE is deliberately tenant-scoped and shared across runs
//!   within one tenant. Two host builds minted from the same runtime share the
//!   same predicate counters — by design.
//! - What *is* per-run-fresh is the **dispatcher** itself: each `rebuild` mints
//!   a fresh [`HookDispatcherBuilder`]/registry so slot-poisoning and registry
//!   mutations never leak across runs.
//!
//! That split — per-run-fresh dispatcher, tenant-scoped predicate counters — is
//! the documented isolation boundary. The
//! `predicate_counter_state_is_tenant_scoped_across_rebuilds` test pins the
//! shared-across-runs half; `rebuild_mints_independent_dispatchers_per_call`
//! pins the fresh-dispatcher half. Both must hold.
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
use ironclaw_hooks::dispatch::HookDispatcherBuilder;
use ironclaw_hooks::evaluator::PredicateEvaluator;
use ironclaw_hooks::manifest::HookManifestEntry;
use ironclaw_hooks::predicate_state::{InMemoryPredicateStateBackend, PredicateStateBackend};
use ironclaw_hooks::registrar::HookRegistrar;
use ironclaw_hooks::registry::HookRegistry;

use crate::error::RebornBuildError;

/// Per-host-build factory closure passed to
/// `RebornLoopDriverHostFactory::with_hook_dispatcher_builder_factory`. The
/// closure is invoked once per `build_text_only_host*` call and returns a
/// fresh [`HookDispatcherBuilder`] (no pre-attached milestone sink — the host
/// factory wires a run-scoped one).
///
/// Re-exported from `ironclaw_reborn` so the type is identical to the one
/// `DefaultPlannedRuntimeParts::hook_dispatcher_builder_factory` accepts; this
/// crate just gives it a local name at its public surface.
pub use ironclaw_reborn::loop_driver_host::HookDispatcherBuilderFactory;

/// Activation configuration for the hook framework.
///
/// **Default OFF.** This is the rollout-safety contract: a default-constructed
/// config (or one built from an unset environment) leaves the dispatcher
/// uncomposed, so the production runtime behaves exactly as it did before
/// hooks existed. The flag is flipped to ON deliberately (canary → on), never
/// by accident.
// `#[derive(Default)]` gives `enabled: false` (bool's default) — i.e. OFF.
// The default-OFF contract is load-bearing; the `config_defaults_to_disabled`
// test pins it so the derive can never silently flip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HooksActivationConfig {
    enabled: bool,
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

/// Install the first-party builtin hook set into `builder`.
///
/// Builtin hooks are `Builtin`-tier (full authority within the framework) and
/// are identified by a stable canonical path, not a content-addressed
/// extension id. They are installed regardless of which extensions are
/// present.
///
/// **The production catalog is empty.** No real first-party builtin hook has
/// been productized, so this installs nothing and returns the builder
/// unchanged. An empty first-party set is a legitimate composed state — the
/// activation machinery below composes a valid (possibly zero-binding)
/// dispatcher with the flag ON. First-party hooks are added here when (and
/// only when) a real one exists; the machinery is otherwise exercised with
/// test-only hooks through the [`build_hook_dispatcher_builder_factory_with`]
/// seam (see tests).
fn install_first_party_hooks(
    builder: HookDispatcherBuilder,
) -> Result<HookDispatcherBuilder, RebornBuildError> {
    // Empty production catalog. See the module docs (item 2) for why no no-op
    // hook is shipped here.
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
///
/// # Activation scope (what is live in production today)
///
/// This loader fully supports installed-tier hooks declared by *any*
/// extension package in `registry`. The production composition root
/// ([`crate::runtime::build_reborn_runtime`]) invokes
/// [`build_hook_dispatcher_builder_factory`] with the **canonical** extension
/// registry — the same `Arc<ExtensionRegistry>` that
/// `HostRuntimeServices::new` resolves capability dispatch through (carried as
/// a shared composition artifact through `RebornLocalRuntimeServices`), not a
/// freshly-rebuilt builtin-only sidecar. Today that canonical registry carries
/// only the builtin / host-bundled first-party package, and the first-party
/// builtin catalog ([`install_first_party_hooks`]) is **empty**, so with the
/// flag ON the live runtime activates exactly one hook source:
///
/// - `[[hooks]]` declared by builtin / host-bundled packages.
///
/// (When a real first-party builtin hook is productized it joins this list; the
/// activation machinery for it is already in place and exercised by test-only
/// hooks.)
///
/// Hooks declared by third-party *installed* extensions are **not** yet
/// surfaced into the runtime path — not because this loader can't install them
/// (it can), but because the canonical registry does not yet carry installed
/// third-party packages. Because hook activation now reads that *same*
/// registry, the moment installed extensions are inserted into it upstream
/// they flow through here with no change to the call site. "Extension-declared
/// hooks" in this module therefore means "builtin-package-declared hooks" in
/// production until the canonical registry carries installed packages. Live
/// third-party activation is a deliberate follow-up that follows the registry,
/// not a separate wiring path.
fn project_extension_install_sets(
    registry: &ExtensionRegistry,
) -> Result<Vec<ExtensionInstallSet>, RebornBuildError> {
    let mut sets = Vec::new();
    for package in registry.extensions() {
        let manifest = &package.manifest;
        if manifest.hooks.is_empty() {
            continue;
        }
        let mut entries = Vec::with_capacity(manifest.hooks.len());
        for hook in &manifest.hooks {
            // Re-parse the canonical TOML the v2 parser preserved into the
            // typed hook entry. This is the clean-boundary projection: the
            // extension crate never knew the hook vocabulary; we do. This is
            // the only fallible, external-input step in the whole loader.
            let entry: HookManifestEntry = toml::from_str(&hook.raw_toml).map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!(
                        "extension `{}` hook `{}` is not a valid hook manifest entry: {error}",
                        manifest.id.as_str(),
                        hook.local_id
                    ),
                }
            })?;
            entries.push(entry);
        }
        sets.push(ExtensionInstallSet {
            extension_id: manifest.id.clone(),
            extension_version: manifest.version.clone(),
            entries,
        });
    }
    Ok(sets)
}

/// Install pre-projected, typed extension install sets through `registrar` at
/// the `Installed` trust tier. Shared by plan validation and per-run rebuild,
/// so there is exactly one extension-install code path. Returns the builder
/// carrying the installed bindings.
fn install_extension_sets(
    sets: &[ExtensionInstallSet],
    registrar: &HookRegistrar,
    mut builder: HookDispatcherBuilder,
) -> Result<HookDispatcherBuilder, RebornBuildError> {
    for set in sets {
        let (next, _hook_ids) = registrar
            .install(
                set.extension_id.clone(),
                set.extension_version.clone(),
                set.entries.clone(),
                builder,
            )
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!(
                    "failed to install hooks declared by extension `{}`: {error}",
                    set.extension_id.as_str()
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
/// - **Flag ON** ⇒ compiles a [`HookInstallPlan`] once (parse every extension's
///   `[[hooks]]` TOML into typed entries + validate the full install set against
///   a fresh empty builder, fail-closed on any error), then returns a closure
///   that calls [`HookInstallPlan::rebuild`] to mint a fresh
///   [`HookDispatcherBuilder`] per host build. The fresh-per-build construction
///   gives each run its own dispatcher (no cross-run slot-poison/registry
///   leak). Predicate counter state is the deliberate exception — it is
///   tenant-scoped and shared across runs (see the module "Predicate counter
///   scoping" note).
///
/// `registry` must be the per-tenant extension registry for the runtime being
/// composed (in production, the canonical `Arc<ExtensionRegistry>` that
/// `HostRuntimeServices::new` resolves capability dispatch through). The plan
/// captures the validated typed inputs, not a built dispatcher, so each
/// `rebuild` produces an independent dispatcher.
pub fn build_hook_dispatcher_builder_factory(
    config: HooksActivationConfig,
    registry: &ExtensionRegistry,
) -> Result<Option<HookDispatcherBuilderFactory>, RebornBuildError> {
    // Production path: the first-party catalog is empty
    // (`install_first_party_hooks` is a no-op). All other wiring lives in the
    // shared helper.
    build_hook_dispatcher_builder_factory_with(config, registry, install_first_party_hooks)
}

/// Per-extension typed install record captured once at plan-construction so
/// the per-run rebuild never re-parses TOML (the only fallible, external-input
/// step). `HookManifestEntry` is cheap to clone relative to per-run dispatch.
struct ExtensionInstallSet {
    extension_id: ironclaw_host_api::ExtensionId,
    extension_version: String,
    entries: Vec<HookManifestEntry>,
}

/// A hook install set that has been **parsed once and validated once** against
/// a fresh empty builder, fail-closed.
///
/// This is the single typed artifact behind the per-run dispatcher factory.
/// Constructing it is the *only* fallible step ([`HookInstallPlan::compile`]):
/// it projects every extension's `[[hooks]]` TOML into typed entries, then
/// proves the entire install set composes a valid dispatcher by running it
/// against a scratch builder (any failure surfaces as a `RebornBuildError`).
///
/// Once compiled, [`HookInstallPlan::rebuild`] mints a fresh
/// [`HookDispatcherBuilder`] per run. That rebuild is **infallible by
/// construction**: it replays the *exact same* install sequence from the
/// *exact same* fresh-empty starting state that `compile` already proved
/// succeeds, and the registrar install is a pure deterministic function of
/// `(entries, fresh-empty builder, evaluator)`. There is therefore no
/// per-run error path — no `.expect()`, no prose-justified `unwrap`. The
/// invariant is carried by the type: a `HookInstallPlan` can only exist if its
/// install set already composed cleanly.
struct HookInstallPlan {
    /// The first-party installer, replayed verbatim per run. Held as a boxed
    /// `Fn` so production (empty catalog) and tests (a test-only hook) share
    /// one plan shape. Validated by `compile` against a scratch builder.
    install_first_party: Box<
        dyn Fn(HookDispatcherBuilder) -> Result<HookDispatcherBuilder, RebornBuildError>
            + Send
            + Sync,
    >,
    /// Per-extension typed entries, projected + validated once.
    extension_install_sets: Vec<ExtensionInstallSet>,
    /// Shared per-tenant predicate evaluator. Intentionally captured ONCE and
    /// reused across every per-run rebuild — see the module-level
    /// "Predicate counter scoping" note: predicate rate/value-cap counters are
    /// deliberately tenant-scoped, not run-scoped.
    evaluator: Arc<PredicateEvaluator>,
}

impl HookInstallPlan {
    /// Parse + validate the full install set ONCE, fail-closed. Returns the
    /// compiled plan whose per-run [`rebuild`](Self::rebuild) is infallible by
    /// construction. An empty install set (empty first-party catalog + no
    /// extension hooks) is a legitimate state — it compiles a zero-binding
    /// plan, not an error.
    fn compile<F>(
        registry: &ExtensionRegistry,
        evaluator: Arc<PredicateEvaluator>,
        install_first_party: F,
    ) -> Result<Self, RebornBuildError>
    where
        F: Fn(HookDispatcherBuilder) -> Result<HookDispatcherBuilder, RebornBuildError>
            + Send
            + Sync
            + 'static,
    {
        // Project every extension's `[[hooks]]` TOML into typed entries once
        // (the only fallible, external-input step; fails the build closed on a
        // malformed manifest).
        let extension_install_sets = project_extension_install_sets(registry)?;

        let plan = Self {
            install_first_party: Box::new(install_first_party),
            extension_install_sets,
            evaluator,
        };

        // Prove the full install set composes a valid dispatcher against a
        // fresh empty builder. If this succeeds, every future `rebuild` —
        // which replays the identical sequence from the identical fresh-empty
        // start — succeeds too, which is what makes `rebuild` infallible. We
        // discard the validated builder; `rebuild` produces fresh ones per run
        // so no `Box<dyn Hook>` state leaks across runs.
        let _validated = plan.try_build_once()?;

        Ok(plan)
    }

    /// Run the install sequence against a fresh empty builder, fallibly.
    ///
    /// Used by [`compile`](Self::compile) to validate the plan. Not called per
    /// run — [`rebuild`](Self::rebuild) is the per-run entry point and is
    /// infallible because `compile` already proved this exact sequence
    /// succeeds.
    fn try_build_once(&self) -> Result<HookDispatcherBuilder, RebornBuildError> {
        let builder = HookDispatcherBuilder::new(HookRegistry::new());
        let builder = (self.install_first_party)(builder)?;
        // A fresh registrar per build: it only wraps `Arc<evaluator>`, so this
        // is cheap and keeps no cross-build state of its own.
        let registrar = HookRegistrar::new(Arc::clone(&self.evaluator));
        install_extension_sets(&self.extension_install_sets, &registrar, builder)
    }

    /// Mint a fresh [`HookDispatcherBuilder`] for one run.
    ///
    /// Infallible by construction: this replays the exact install sequence
    /// from the exact fresh-empty starting state that [`compile`](Self::compile)
    /// already proved succeeds (fail-closed via `?` at compile time). The
    /// registrar install is a pure deterministic function of
    /// `(entries, fresh-empty builder, evaluator)`, so a replay cannot reach an
    /// error the compile-time validation didn't. The `unreachable!` therefore
    /// guards a true logic invariant carried by the type — a `HookInstallPlan`
    /// only exists for an install set that compiled cleanly — not an error
    /// that could occur in practice.
    fn rebuild(&self) -> HookDispatcherBuilder {
        match self.try_build_once() {
            Ok(builder) => builder,
            Err(error) => unreachable!(
                "HookInstallPlan::rebuild replayed an install set that \
                 HookInstallPlan::compile already validated against an \
                 identical fresh-empty builder; a deterministic replay cannot \
                 fail where validation passed. Underlying error: {error}"
            ),
        }
    }
}

/// Shared implementation behind [`build_hook_dispatcher_builder_factory`],
/// parameterized on the first-party install step.
///
/// `install_first_party` is invoked at compile-time validation and on every
/// per-run rebuild, so it must be a pure replayable function of its builder
/// input. Production passes [`install_first_party_hooks`] (the empty catalog);
/// tests pass a closure that installs a test-only first-party hook, exercising
/// the activation machinery end-to-end through the real composition path
/// without shipping a production no-op.
fn build_hook_dispatcher_builder_factory_with<F>(
    config: HooksActivationConfig,
    registry: &ExtensionRegistry,
    install_first_party: F,
) -> Result<Option<HookDispatcherBuilderFactory>, RebornBuildError>
where
    F: Fn(HookDispatcherBuilder) -> Result<HookDispatcherBuilder, RebornBuildError>
        + Send
        + Sync
        + 'static,
{
    if !config.is_enabled() {
        return Ok(None);
    }

    // In-memory predicate-state backend for v1. Swappable: a durable
    // Postgres/libSQL backend (#3933) drops in here without touching the rest
    // of the wiring. The evaluator is built ONCE per runtime and shared across
    // every per-run rebuild — predicate counters are deliberately tenant-scoped
    // (see the module-level "Predicate counter scoping" note).
    let backend: Arc<dyn PredicateStateBackend> = Arc::new(InMemoryPredicateStateBackend::new());
    let evaluator = Arc::new(PredicateEvaluator::with_state_backend(Arc::clone(&backend)));
    evaluator.warn_in_memory_backend_active_in_production();

    // Parse ONCE + validate ONCE into a typed plan, fail-closed. After this,
    // the per-run factory closure is infallible by construction.
    let plan = HookInstallPlan::compile(registry, evaluator, install_first_party)?;

    let factory: HookDispatcherBuilderFactory = Arc::new(move || plan.rebuild());

    Ok(Some(factory))
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_extensions::v2::ManifestSource;
    use ironclaw_extensions::{ExtensionManifest, ExtensionPackage};
    use ironclaw_hooks::HookPhase;
    use ironclaw_hooks::identity::{HookId, HookVersion};
    use ironclaw_hooks::points::ObserverHookContext;
    use ironclaw_hooks::registry::HookPointSpec;
    use ironclaw_hooks::sink::{ObserverHook, ObserverSink};
    use ironclaw_host_api::{HostPortCatalog, VirtualPath};

    /// Canonical identity path for the TEST-ONLY first-party no-op observer.
    /// Lives here (not in the production catalog) so the activation machinery
    /// can be exercised end-to-end through the real composition path without
    /// shipping a no-op hook in production.
    const TEST_NOOP_OBSERVER_CANONICAL_PATH: &str =
        "ironclaw_reborn_composition::hooks::tests::NoOpObserverHook";

    /// A test-only first-party no-op observer. Observers cannot affect
    /// outcomes; this one records nothing. It proves the builtin install +
    /// dispatch path end to end through `build_hook_dispatcher_builder_factory_with`.
    #[derive(Debug, Default)]
    struct NoOpObserverHook;

    #[async_trait::async_trait]
    impl ObserverHook for NoOpObserverHook {
        async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {}
    }

    /// Test-only first-party installer: installs the [`NoOpObserverHook`] at
    /// `AfterCapability`. Passed to
    /// [`build_hook_dispatcher_builder_factory_with`] in place of the empty
    /// production catalog so the activation machinery is covered with a real
    /// first-party binding.
    fn install_test_first_party_hook(
        builder: HookDispatcherBuilder,
    ) -> Result<HookDispatcherBuilder, RebornBuildError> {
        let hook_id = HookId::for_builtin(TEST_NOOP_OBSERVER_CANONICAL_PATH, HookVersion::ONE);
        builder
            .install_builtin_observer(
                hook_id,
                HookPhase::Telemetry,
                HookPointSpec::AfterCapability,
                Box::new(NoOpObserverHook),
            )
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("failed to install test first-party no-op observer hook: {error}"),
            })
    }

    /// Build a single-capability `reborn.extension_manifest.v2` manifest TOML
    /// for `id`, optionally carrying a `[[hooks]]` block. The capability id is
    /// provider-prefixed (`<id>.run`) as `ExtensionPackage::from_manifest`
    /// requires.
    fn manifest_toml(id: &str, hooks_block: &str) -> String {
        format!(
            r#"schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "{id} extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

[[capabilities]]
id = "{id}.run"
description = "Run {id}"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/{id}/run.input.v1.json"
output_schema_ref = "schemas/{id}/run.output.v1.json"
prompt_doc_ref = "prompts/{id}/run.md"
{hooks_block}"#
        )
    }

    /// Parse `toml` into a validated package rooted at the conventional
    /// extensions path and insert it into a fresh registry.
    fn registry_with_manifest(id: &str, toml: &str) -> ExtensionRegistry {
        let manifest = ExtensionManifest::parse(
            toml,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
        )
        .expect("manifest parses");
        let package = ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new(format!("/system/extensions/{id}")).expect("valid root path"),
        )
        .expect("package builds from manifest");
        let mut registry = ExtensionRegistry::new();
        registry.insert(package).expect("package inserts");
        registry
    }

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
    fn enabled_config_with_empty_production_catalog_yields_valid_zero_binding_factory() {
        // The PRODUCTION first-party catalog is empty. Flag ON + empty
        // first-party set + no extension hooks must still compose a valid
        // dispatcher — a zero-binding dispatcher, not a panic/error. This pins
        // the empty-catalog-is-valid contract.
        let registry = ExtensionRegistry::new();
        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("enabled build with empty registry + empty catalog succeeds")
                .expect("flag ON yields a factory even with an empty catalog");
        // The factory mints a valid dispatcher with no first-party bindings.
        let dispatcher = factory().build_arc();
        let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::AfterCapability);
        assert!(
            bindings.is_empty(),
            "empty production catalog must yield zero first-party bindings, saw {bindings:?}"
        );
    }

    #[test]
    fn activation_installs_a_test_first_party_hook_through_the_real_path() {
        // The activation machinery is exercised end-to-end with a TEST-ONLY
        // first-party hook (not a production-shipped no-op). We drive the same
        // composition path via the `*_with` seam and confirm the test hook is
        // bound at AfterCapability.
        let registry = ExtensionRegistry::new();
        let factory = build_hook_dispatcher_builder_factory_with(
            HooksActivationConfig::enabled(),
            &registry,
            install_test_first_party_hook,
        )
        .expect("enabled build with a test first-party hook succeeds")
        .expect("flag ON yields a factory");
        let dispatcher = factory().build_arc();
        let test_id = HookId::for_builtin(TEST_NOOP_OBSERVER_CANONICAL_PATH, HookVersion::ONE);
        let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::AfterCapability);
        assert!(
            bindings.iter().any(|binding| binding.hook_id == test_id),
            "test-only first-party hook must be installed through the real composition path"
        );
    }

    /// Per-run-fresh DISPATCHER boundary: each `rebuild` mints a distinct
    /// `HookDispatcher` (distinct `Arc`), so dispatcher-local state
    /// (slot-poisoning, registry edits) cannot leak across runs. This proves
    /// only dispatcher freshness — NOT predicate counter scoping, which is the
    /// separate (tenant-scoped, shared-across-runs) boundary pinned by
    /// `predicate_counter_state_is_tenant_scoped_across_rebuilds`.
    #[test]
    fn rebuild_mints_independent_dispatchers_per_call() {
        let registry = ExtensionRegistry::new();
        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("enabled build succeeds")
                .expect("flag ON yields a factory");
        let a = factory().build_arc();
        let b = factory().build_arc();
        assert!(
            !Arc::ptr_eq(&a, &b),
            "each factory call must mint a fresh dispatcher (per-run dispatcher isolation)"
        );
    }

    /// Predicate counter state is TENANT-scoped and deliberately SHARED across
    /// runs (the other half of the isolation boundary documented in the module
    /// "Predicate counter scoping" note). This drives the real composition
    /// path: an extension declares an `InvocationCount { max = 1 }` rate cap;
    /// we build the per-run factory once, then mint TWO dispatchers from it
    /// (two runs). A `before_capability` dispatch through the FIRST dispatcher
    /// records one invocation; a dispatch through the SECOND, freshly-rebuilt
    /// dispatcher must then be DENIED because it sees the count the first run
    /// recorded. If the evaluator/backend were per-run instead of tenant-scoped,
    /// the second dispatcher would start from zero and allow — the assertion
    /// below would fail. This is the regression guard for "two host builds from
    /// the same runtime share the predicate counter".
    #[tokio::test]
    async fn predicate_counter_state_is_tenant_scoped_across_rebuilds() {
        use ironclaw_hooks::points::{BeforeCapabilityHookContext, SanitizedArguments};
        use ironclaw_host_api::{ExtensionId as HostExtensionId, TenantId};

        // `max = 1`: the first matching invocation is allowed (count 0 -> 1);
        // any later invocation in the window is denied (count already at cap).
        let hooks_block = r#"
[[hooks]]
id = "cap-run"
kind = "before_capability"
scope = "own_capabilities"
body = { mode = "predicate", spec = { type = "rate_or_value_cap", when = { type = "name_equals", name = "ratecap-ext.run" }, bound = { type = "invocation_count", max = 1, window = "24h" }, on_exceeded = { decision = "deny", reason = "rate cap reached" } } }
"#;
        let registry =
            registry_with_manifest("ratecap-ext", &manifest_toml("ratecap-ext", hooks_block));

        // Build the per-run factory ONCE — the evaluator/backend is captured
        // here and shared across every rebuild (tenant-scoped by construction).
        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("enabled build with a rate-cap extension hook succeeds")
                .expect("flag ON yields a factory");

        let tenant = TenantId::new("tenant-a").expect("valid tenant id");
        let provider = HostExtensionId::new("ratecap-ext").expect("valid provider id");
        let make_ctx = || {
            BeforeCapabilityHookContext::new(
                tenant.clone(),
                "ratecap-ext.run".to_string(),
                [0u8; 32],
                SanitizedArguments::unresolved(),
                Some(provider.clone()),
            )
        };

        // Run 1: fresh dispatcher; first invocation is under the cap -> allowed.
        let run_one = factory().build_arc();
        let first = run_one.dispatch_before_capability(&make_ctx()).await;
        assert!(
            first.decision.permits(),
            "first invocation under an InvocationCount(max=1) cap must be allowed, \
             got {:?}",
            first.decision.view()
        );

        // Run 2: a SEPARATE rebuild (distinct dispatcher Arc) that nonetheless
        // shares the tenant-scoped predicate counter. The count recorded by run
        // 1 is now at the cap, so this invocation must be DENIED.
        let run_two = factory().build_arc();
        assert!(
            !Arc::ptr_eq(&run_one, &run_two),
            "the two runs must be distinct dispatchers (per-run freshness holds)"
        );
        let second = run_two.dispatch_before_capability(&make_ctx()).await;
        assert!(
            !second.decision.permits(),
            "a fresh rebuild must observe the invocation count recorded by the \
             previous run (tenant-scoped predicate counter shared across runs); \
             it was allowed instead, which means the counter reset per run: {:?}",
            second.decision.view()
        );
    }

    // ─── Direct composition-loader coverage ──────────────────────────────────
    //
    // The tests above exercise the activation seam with an empty registry
    // (first-party-only). The tests below drive the extension-hook loader
    // (`project_extension_install_sets` + `install_extension_sets`) through
    // `build_hook_dispatcher_builder_factory` with a real `ExtensionRegistry`
    // carrying `[[hooks]]` declarations, so the full `ExtensionManifestV2`
    // `[[hooks]]` DTO → `HookManifestEntry` → registrar path is covered — not a
    // loader look-alike. The load-bearing case is
    // `malformed_extension_hook_manifest_fails_closed_not_panics`: an
    // attacker-controlled installed manifest must degrade to a `RebornBuildError`,
    // never a panic.

    /// A registry package declaring a VALID `own_capabilities` predicate hook
    /// installs at the `Installed` trust tier, and the resulting dispatcher
    /// carries a binding for that extension's hook id at the
    /// `BeforeCapability` point alongside a (test-only) first-party hook —
    /// proving the extension install does not displace the first-party set.
    /// Driven through the `*_with` seam so the first-party hook is test-only,
    /// not a production-shipped no-op.
    #[test]
    fn valid_extension_hook_manifest_installs_at_installed_tier() {
        use ironclaw_hooks::identity::{ExtensionId as HookExtensionId, HookLocalId};

        // `own_capabilities` scope requires no user grant, so the loader's
        // registrar (empty verified-grants set) installs it cleanly. A
        // declarative predicate body needs no WASM runtime.
        let hooks_block = r#"
[[hooks]]
id = "deny-run"
kind = "before_capability"
scope = "own_capabilities"
body = { mode = "predicate", spec = { type = "deny_capability", reason = "blocked by manifest hook", when = { type = "name_equals", name = "valid-ext.run" } } }
"#;
        let registry =
            registry_with_manifest("valid-ext", &manifest_toml("valid-ext", hooks_block));

        let factory = build_hook_dispatcher_builder_factory_with(
            HooksActivationConfig::enabled(),
            &registry,
            install_test_first_party_hook,
        )
        .expect("enabled build with a valid extension hook succeeds")
        .expect("flag ON yields a factory");
        let dispatcher = factory().build_arc();

        // The extension hook id is derived deterministically from the
        // extension id + version + local id, exactly as the registrar mints
        // it. Assert the dispatcher carries that binding at BeforeCapability.
        let expected = HookId::derive(
            &HookExtensionId::new("valid-ext").expect("valid extension id"),
            "0.1.0",
            &HookLocalId::new("deny-run").expect("valid hook local id"),
            HookVersion::ONE,
        );
        let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::BeforeCapability);
        assert!(
            bindings.iter().any(|binding| binding.hook_id == expected),
            "installed extension hook must be bound at BeforeCapability; saw {bindings:?}"
        );

        // The test-only first-party hook still rides along at AfterCapability —
        // the extension install does not displace the first-party set.
        let test_id = HookId::for_builtin(TEST_NOOP_OBSERVER_CANONICAL_PATH, HookVersion::ONE);
        let after = dispatcher.active_bindings_snapshot(HookPointSpec::AfterCapability);
        assert!(
            after.iter().any(|binding| binding.hook_id == test_id),
            "test-only first-party hook must remain installed alongside extension hooks"
        );
    }

    /// A malformed typed hook payload (the body declares an unknown `mode`)
    /// must fail the build CLOSED with a `RebornBuildError::InvalidConfig`,
    /// never a panic. This is the load-bearing degradation contract: external
    /// manifests are untrusted input and a bad one cannot crash composition.
    #[test]
    fn malformed_extension_hook_manifest_fails_closed_not_panics() {
        // `mode = "nonsense"` is not a recognized `HookManifestBody` variant,
        // so the loader's `toml::from_str::<HookManifestEntry>` projection
        // rejects it. The whole build must fail with InvalidConfig.
        let hooks_block = r#"
[[hooks]]
id = "broken-hook"
kind = "before_capability"
body = { mode = "nonsense" }
"#;
        let registry =
            registry_with_manifest("broken-ext", &manifest_toml("broken-ext", hooks_block));

        let result =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry);
        match result {
            Err(RebornBuildError::InvalidConfig { reason }) => {
                assert!(
                    reason.contains("broken-ext") && reason.contains("broken-hook"),
                    "fail-closed error must name the offending extension + hook, got: {reason}"
                );
            }
            Ok(_) => panic!("malformed manifest must fail closed, but the build succeeded"),
            Err(other) => panic!(
                "malformed manifest must fail with InvalidConfig, got a different error: {other}"
            ),
        }
    }

    /// A hook declaring `scope = same_tenant` reaches beyond the declaring
    /// extension's own capabilities and therefore requires an explicit user
    /// grant. The composition loader's registrar carries no verified grants,
    /// so an installed-tier extension hook claiming that wider scope is
    /// rejected (trust attenuation) — fail-closed, not a panic.
    #[test]
    fn extension_hook_claiming_ungranted_wider_scope_is_rejected() {
        // `same_tenant` scope requires `requires_grant`; the manifest sets it,
        // but the loader's registrar has no matching verified grant, so the
        // install is denied. (Omitting `requires_grant` would instead fail the
        // entry's own `validate()`; either way the build must fail closed.)
        let hooks_block = r#"
[[hooks]]
id = "cross-tenant-deny"
kind = "before_capability"
scope = "same_tenant"
requires_grant = "cross-tenant-policy"
body = { mode = "predicate", spec = { type = "deny_capability", reason = "wider-scope deny", when = { type = "name_equals", name = "other-ext.run" } } }
"#;
        let registry =
            registry_with_manifest("reachy-ext", &manifest_toml("reachy-ext", hooks_block));

        let result =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry);
        match result {
            Err(RebornBuildError::InvalidConfig { reason }) => {
                assert!(
                    reason.contains("reachy-ext"),
                    "trust-attenuation rejection must name the extension, got: {reason}"
                );
            }
            Ok(_) => {
                panic!("ungranted wider-scope hook must be rejected, but the build succeeded")
            }
            Err(other) => panic!(
                "ungranted wider-scope hook must be rejected with InvalidConfig, got a different error: {other}"
            ),
        }
    }
}
