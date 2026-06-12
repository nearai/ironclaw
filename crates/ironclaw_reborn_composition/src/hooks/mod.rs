//! Production activation of the hook framework.
//!
//! This module is the single composition seam that flips the (otherwise
//! dormant) hook framework into the live capability-invocation path. It owns:
//!
//! 1. **The feature flag** ([`HooksActivationConfig`]) — default OFF. When
//!    OFF, [`build_hook_dispatcher_builder_factory`] returns `None` and the
//!    runtime composes exactly as it did before hooks existed (zero behavior
//!    change). This is the hard rollout-safety contract for the activation.
//! 2. **The first-party builtin hook set** ([`factory::install_first_party_hooks`]) —
//!    installed regardless of extensions. The production catalog is
//!    deliberately **EMPTY**: no real first-party builtin hook has been
//!    productized, so we ship none. A production type + install path + behavior
//!    for a hook that does nothing is scaffolding, not a deliverable. The
//!    activation machinery is exercised end-to-end with test-only hooks (see
//!    the `#[cfg(test)]` `NoOpObserverHook` and the test-only first-party
//!    installer seam in [`tests`]), not with a shipped no-op. An empty
//!    first-party set composed with no extension hooks is a legitimate state —
//!    the dispatcher composes with zero bindings, which is valid (not a
//!    panic/error).
//! 3. **The manifest → registry loader** ([`factory::project_extension_hook_sets`])
//!    — takes the hook-only [`projection::HookProjection`] of each installed
//!    extension, projects each declared `[[hooks]]` payload into a typed
//!    [`ironclaw_hooks::manifest::HookManifestEntry`], and installs them
//!    through [`ironclaw_hooks::registrar::HookRegistrar::install`] at the
//!    `Installed` trust tier. Trust attenuation is enforced by construction:
//!    the registrar only ever calls `install_installed_*`, so an extension hook
//!    can never mint `Allow` / `Gate` / `Mutator` without an explicit
//!    per-extension grant.
//! 4. **The per-run dispatcher builder factory** — returned to the runtime to
//!    pass to `RebornLoopDriverHostFactory::with_hook_dispatcher_builder_
//!    factory`. The closure mints a *fresh* [`HookDispatcherBuilder`] per host
//!    build (per run), so slot-poisoning and registry mutations never leak
//!    across runs. Telemetry attribution is per-run because the host factory
//!    attaches the run-scoped milestone sink internally to each fresh builder.
//!
//! ## Module layout (#3951 P1 finding #4 decomposition)
//!
//! The activation logic is split into focused submodules, behavior-preserving:
//!
//! - [`projection`] — the hook-only [`projection::HookProjection`] /
//!   [`HookProjectionRegistry`] containment types, plus the discovery/admission
//!   pipeline ([`projection::build_hook_projection_registry`]) and the
//!   tenant-root / path-containment helpers.
//! - [`factory`] — first-party install, install-time per-extension quarantine
//!   validation ([`factory::project_extension_hook_sets`]), and the
//!   fresh-per-build dispatcher factory builders.
//! - [`audit`] — `hook.quarantined` security-audit emission.
//! - `tests` — the unit-test matrix (test-only; compiled under `cfg(test)`).
//!
//! ## Per-tenant scoping (multi-tenant isolation contract, #3890)
//!
//! `build_reborn_runtime` is invoked once per identity/owner — one
//! `tenant_id` per call. Everything this module constructs (the
//! [`ironclaw_hooks::evaluator::PredicateEvaluator`] + its state backend, the
//! template registry, the per-run dispatcher closure) is built inside that
//! per-tenant call, so one tenant's hooks and predicate counters can never
//! apply to another. There is no global registry.
//!
//! ## Predicate backend (in-memory for v1)
//!
//! The evaluator uses the in-memory predicate-state backend for now. The
//! backend is swappable
//! ([`ironclaw_hooks::evaluator::PredicateEvaluator::with_state_backend`]) so
//! the durable Postgres/libSQL backends (#3933 + follow-ups) can drop in
//! without touching this module's wiring.

pub(crate) mod audit;
pub(crate) mod factory;
pub(crate) mod projection;

#[cfg(test)]
mod tests;

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

// Public surface of the activation path (consumed by `crate::runtime`).
pub(crate) use factory::build_hook_dispatcher_builder_factory_for_tenant_with_audit_sink;
pub use factory::{
    build_hook_dispatcher_builder_factory, build_hook_dispatcher_builder_factory_for_tenant,
};
pub(crate) use projection::build_hook_projection_registry_with_audit_sink;
pub use projection::{
    HookProjectionRegistry, MAX_INSTALLED_EXTENSIONS_CONSIDERED, MAX_TOTAL_HOOKS_PER_TENANT,
    ThirdPartyDiscoveryInput, build_hook_projection_registry, tenant_extension_root,
};

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
    /// Sub-flag gating *third-party installed-extension* hook activation. The
    /// master `enabled` flag activates builtin/host-bundled hooks; this
    /// additional flag must ALSO be on before any third-party `[[hooks]]`
    /// declaration is discovered and projected. Default OFF: with this off the
    /// activation path is byte-identical to the builtin-only #3938 behavior.
    ///
    /// **Production-enablement gate (read before flipping in production):**
    /// `HOOKS_THIRD_PARTY_ENABLED` MUST NOT be enabled in multi-tenant
    /// production until BOTH prerequisites land:
    ///
    /// 1. **Filesystem hardening:** an `openat2(RESOLVE_BENEATH)` / `O_NOFOLLOW`
    ///    backend. v1 ships a projection-layer strict-child / no-`..` /
    ///    no-symlink-escape containment check plus the canonicalizing local
    ///    backend as the interim mitigation; the hardened backend is the
    ///    documented gating follow-up.
    /// 2. **Durable quarantine surfacing:** `hook.quarantined` security-audit
    ///    events must be wired to a durable sink, not only tracing. The Reborn
    ///    runtime composition wires this sink; future host profiles must
    ///    preserve that wiring before enabling third-party hooks.
    third_party_enabled: bool,
}

/// Environment variable that flips the hook framework on. Absent / empty /
/// any value other than a recognized truthy token ⇒ OFF.
pub const HOOKS_ENABLED_ENV: &str = "HOOKS_ENABLED";

/// Environment variable that additionally flips *third-party installed
/// extension* hook activation on. Requires [`HOOKS_ENABLED_ENV`] to also be
/// truthy. Absent / empty / non-truthy ⇒ OFF.
pub const HOOKS_THIRD_PARTY_ENABLED_ENV: &str = "HOOKS_THIRD_PARTY_ENABLED";

impl HooksActivationConfig {
    /// Explicitly enabled (master flag only; third-party still OFF).
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            third_party_enabled: false,
        }
    }

    /// Explicitly disabled (the default).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            third_party_enabled: false,
        }
    }

    /// Builder: turn the third-party sub-flag on. Has no effect unless the
    /// master flag is also on (see [`Self::is_third_party_enabled`]).
    #[must_use]
    pub fn with_third_party_enabled(mut self, third_party_enabled: bool) -> Self {
        self.third_party_enabled = third_party_enabled;
        self
    }

    /// Resolve the activation flag from the process environment. Fail-safe to
    /// OFF: only the canonical truthy tokens (`1`, `true`, `yes`, `on`,
    /// case-insensitive) enable the framework; everything else — including an
    /// unset variable or an unparseable value — leaves it disabled.
    ///
    /// The third-party sub-flag is resolved from
    /// [`HOOKS_THIRD_PARTY_ENABLED_ENV`] by the same rules; it stays inert
    /// unless the master flag is also on.
    pub fn from_env() -> Self {
        Self::from_env_values(
            ironclaw_common::env_helpers::env_or_override(HOOKS_ENABLED_ENV),
            ironclaw_common::env_helpers::env_or_override(HOOKS_THIRD_PARTY_ENABLED_ENV),
        )
    }

    fn from_env_values(enabled: Option<String>, third_party_enabled: Option<String>) -> Self {
        Self {
            enabled: enabled.as_deref().is_some_and(is_truthy),
            third_party_enabled: third_party_enabled.as_deref().is_some_and(is_truthy),
        }
    }

    pub fn is_enabled(self) -> bool {
        self.enabled
    }

    /// True only when BOTH the master flag and the third-party sub-flag are on.
    /// This is the single gate the projection path consults before discovering
    /// or projecting any third-party `[[hooks]]` declaration.
    pub fn is_third_party_enabled(self) -> bool {
        self.enabled && self.third_party_enabled
    }
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}
