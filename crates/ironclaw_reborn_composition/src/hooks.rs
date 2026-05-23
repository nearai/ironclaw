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
/// extension package in `registry`. However, the production composition root
/// ([`crate::runtime::build_reborn_runtime`]) currently invokes
/// [`build_hook_dispatcher_builder_factory`] with the *builtin* extension
/// registry only (`builtin_extension_registry()`). The first-party builtin
/// catalog ([`install_first_party_hooks`]) is currently **empty**, so with the
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
/// (it can), but because no installed-extension registry is threaded into the
/// call site yet. "Extension-declared hooks" in this module therefore means
/// "builtin-package-declared hooks" in production until that registry is
/// wired. Live third-party activation is a deliberate follow-up.
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
    // Production path: the first-party catalog is empty
    // (`install_first_party_hooks` is a no-op). All other wiring lives in the
    // shared helper.
    build_hook_dispatcher_builder_factory_with(config, registry, install_first_party_hooks)
}

/// Shared implementation behind [`build_hook_dispatcher_builder_factory`],
/// parameterized on the first-party install step.
///
/// `install_first_party` is invoked both at composition-time validation and on
/// every per-run builder mint, so it must be a pure replayable function of its
/// builder input (no external/fallible input beyond what is validated up
/// front). Production passes [`install_first_party_hooks`] (the empty catalog);
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
    // this fails, the build fails loudly (fail-closed). An empty install set
    // (empty first-party catalog + no extension hooks) is a legitimate state —
    // a zero-binding dispatcher composes fine and is valid.
    {
        let scratch = HookDispatcherBuilder::new(HookRegistry::new());
        let scratch = install_first_party(scratch)?;
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
        // safety: `install_first_party` is a pure replayable function of the builder; the identical call was proven to succeed against a scratch builder in the composition-time validation block above (fail-closed via `?`). A per-run replay therefore cannot fail.
        let first_party = "first-party hook install validated at composition time";
        builder = install_first_party(builder).expect(first_party); // safety: replay of composition-validated install; see binding above
        let registrar = HookRegistrar::new(Arc::clone(&evaluator_for_factory));
        for (extension_id, version, entries) in &extension_install_sets {
            // safety: `entries` were already projected from TOML (the only fallible, external-input step) and the same install set was validated against a scratch builder above (fail-closed via `?`). registrar.install is a pure function of these cloned inputs, so the per-run replay cannot fail.
            let ext_install = "extension hook install validated at composition time";
            let (next, _ids) = registrar
                .install(
                    extension_id.clone(),
                    version.clone(),
                    entries.clone(),
                    builder,
                )
                .expect(ext_install); // safety: replay of composition-validated install set; see binding above
            builder = next;
        }
        builder
    });

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

    // ─── Direct composition-loader coverage ──────────────────────────────────
    //
    // The tests above exercise the activation seam with an empty registry
    // (first-party-only). The tests below drive `install_extension_hooks`
    // through `build_hook_dispatcher_builder_factory` with a real
    // `ExtensionRegistry` carrying `[[hooks]]` declarations, so the full
    // `ExtensionManifestV2` `[[hooks]]` DTO → `HookManifestEntry` → registrar
    // path is covered — not a loader look-alike. The load-bearing case is
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
