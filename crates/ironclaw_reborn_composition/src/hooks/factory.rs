//! Per-run hook dispatcher builder factory: first-party install, install-time
//! per-extension quarantine validation, and the fresh-per-build replay closure.
//!
//! This module owns the manifest → registry loader
//! ([`project_extension_hook_sets`]) and the factory builders that the
//! production composition root calls. Trust attenuation is enforced by
//! construction: installs go EXCLUSIVELY through [`HookRegistrar::install`].

use std::sync::Arc;

use ironclaw_events::SecurityAuditSink;
use ironclaw_hooks::dispatch::HookDispatcherBuilder;
use ironclaw_hooks::evaluator::PredicateEvaluator;
use ironclaw_hooks::manifest::HookManifestEntry;
use ironclaw_hooks::predicate_state::{InMemoryPredicateStateBackend, PredicateStateBackend};
use ironclaw_hooks::registrar::HookRegistrar;
use ironclaw_hooks::registry::HookRegistry;

use crate::error::RebornBuildError;

use super::HookDispatcherBuilderFactory;
use super::HooksActivationConfig;
use super::audit::emit_hook_quarantined;
use super::projection::{
    HookProjection, HookProjectionRegistry, MAX_INSTALLED_EXTENSIONS_CONSIDERED,
    MAX_TOTAL_HOOKS_PER_TENANT, enforce_root_containment, tenant_extension_root,
};

/// Build the error returned when a per-run install replay fails inside the
/// [`HookDispatcherBuilderFactory`] closure. Every install set replayed there
/// was already validated against a scratch builder at composition time, so this
/// is unreachable in practice; the typed error (rather than `.expect()`) means a
/// future regression surfaces as a build failure instead of a panic, per the
/// project's no-`.expect()`-in-production rule.
fn reborn_replay_error(
    reason: String,
) -> ironclaw_reborn::loop_driver_host::RebornLoopDriverHostError {
    ironclaw_reborn::loop_driver_host::RebornLoopDriverHostError::InvalidRequest { reason }
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
/// if) one is productized; the install + dispatch path is already exercised
/// end-to-end by test-only hooks.
pub(super) fn install_first_party_hooks(
    builder: HookDispatcherBuilder,
) -> Result<HookDispatcherBuilder, RebornBuildError> {
    // Empty production catalog. See the module docs (item 2) for why no no-op
    // hook is shipped here.
    Ok(builder)
}

/// A surviving extension's projected hook install set: the typed entries that
/// passed scratch validation and are committed to the real builder per run.
/// Deterministic replay material — identical inputs each run.
pub(super) struct ProjectedExtensionHooks {
    pub(super) extension_id: ironclaw_host_api::ExtensionId,
    pub(super) extension_version: String,
    pub(super) entries: Vec<HookManifestEntry>,
}

/// Project the structurally-typed `[[hooks]]` payloads declared by each
/// projection in `projections`, applying **atomic per-extension quarantine**
/// for untrusted (installed/third-party) sources and **fail-closed whole-build**
/// for trusted (builtin / host-bundled) sources.
///
/// This is the manifest → registry loader and the *only* place the
/// `ExtensionManifestV2` hook DTO crosses into the hook crate's typed
/// vocabulary (clean-boundary contract: `ironclaw_extensions` stays free of
/// hook types; the projection happens here, in the crate that depends on both).
///
/// # Trust attenuation (registrar-only invariant)
///
/// Installs go EXCLUSIVELY through [`HookRegistrar::install`]. The registrar is
/// the single seam that (a) installs at the `Installed` trust tier — only ever
/// calling `install_installed_*`, type-level-preventing an extension hook from
/// minting `Allow` / `Gate` / `Mutator` without an explicit verified grant —
/// and (b) derives `owning_extension` from the installer argument, so a
/// manifest cannot spoof a cross-owner attribution. This function MUST NOT call
/// the lower-level `HookDispatcherBuilder::install_installed_*` methods
/// directly: those accept `owning_extension` as a free parameter and would
/// bypass the registrar's ceiling + attribution. An `ironclaw_architecture`
/// source assertion pins this invariant so a future refactor cannot regress it.
///
/// # Atomic quarantine
///
/// For an untrusted extension, the WHOLE hook set is validated against a
/// *scratch* builder first; only if every hook in the set validates is the
/// identical set committed to the real builder. On ANY failure (TOML
/// projection, cap, ungranted scope, WASM body with no runtime, registry
/// validation) the extension's hooks are dropped ENTIRELY, a `hook.quarantined`
/// audit event is emitted, and projection CONTINUES to the next extension. A
/// trusted extension instead fails the whole build (`?` propagation).
///
/// Returns the surviving install sets (the trusted set plus every untrusted set
/// that fully validated), to be replayed deterministically per run.
pub(super) fn project_extension_hook_sets(
    projections: impl Iterator<Item = impl std::ops::Deref<Target = HookProjection>>,
    registrar: &HookRegistrar,
    tenant_id: &ironclaw_host_api::TenantId,
    tenant_root: Option<&ironclaw_host_api::VirtualPath>,
    audit_sink: Option<&Arc<dyn SecurityAuditSink>>,
) -> Result<Vec<ProjectedExtensionHooks>, RebornBuildError> {
    let mut survivors: Vec<ProjectedExtensionHooks> = Vec::new();
    let mut considered = 0usize;
    let mut third_party_hook_total = 0usize;

    for projection in projections {
        let projection = &*projection;
        if projection.hooks.is_empty() {
            continue;
        }
        let trusted = projection.source.allows_first_party();
        let extension_id_str = projection.extension_id.as_str().to_string();
        let hook_count = projection.hooks.len();

        // ── Tenant-wide DoS caps (enforced BEFORE expensive projection). ──
        // Trusted packages are not subject to these caps (they are host-owned
        // and fail-closed-whole-build); only untrusted/third-party packages
        // count against the tenant budget.
        if !trusted {
            considered += 1;
            if considered > MAX_INSTALLED_EXTENSIONS_CONSIDERED {
                emit_hook_quarantined(
                    tenant_id,
                    &extension_id_str,
                    "exceeded MAX_INSTALLED_EXTENSIONS_CONSIDERED",
                    hook_count,
                    audit_sink,
                );
                continue;
            }
            if third_party_hook_total + hook_count > MAX_TOTAL_HOOKS_PER_TENANT {
                emit_hook_quarantined(
                    tenant_id,
                    &extension_id_str,
                    "exceeded MAX_TOTAL_HOOKS_PER_TENANT",
                    hook_count,
                    audit_sink,
                );
                continue;
            }

            // ── Path-containment (FS-hardening v1 defense-in-depth). ──
            if let Some(root) = tenant_root
                && let Err(reason) = enforce_root_containment(root, &projection.root)
            {
                emit_hook_quarantined(
                    tenant_id,
                    &extension_id_str,
                    &reason,
                    hook_count,
                    audit_sink,
                );
                continue;
            }
        }

        // ── Project TOML → typed entries. ──
        let entries = match project_hook_entries(&extension_id_str, &projection.hooks) {
            Ok(entries) => entries,
            Err(reason) => {
                if trusted {
                    return Err(RebornBuildError::InvalidConfig { reason });
                }
                emit_hook_quarantined(
                    tenant_id,
                    &extension_id_str,
                    &reason,
                    hook_count,
                    audit_sink,
                );
                continue;
            }
        };

        let extension_id = projection.extension_id.clone();
        let extension_version = projection.version.clone();

        // ── Validate the WHOLE set against a scratch builder. Commit nothing
        // here; the survivors are replayed against the real builder later. ──
        let scratch = HookDispatcherBuilder::new(HookRegistry::new());
        match registrar.install(extension_id.clone(), &extension_version, &entries, scratch) {
            Ok(_validated) => {
                if !trusted {
                    third_party_hook_total += hook_count;
                }
                survivors.push(ProjectedExtensionHooks {
                    extension_id,
                    extension_version,
                    entries,
                });
            }
            Err(error) => {
                let reason = format!(
                    "failed to install hooks declared by extension `{extension_id_str}`: {error}"
                );
                if trusted {
                    return Err(RebornBuildError::InvalidConfig { reason });
                }
                emit_hook_quarantined(
                    tenant_id,
                    &extension_id_str,
                    &reason,
                    hook_count,
                    audit_sink,
                );
            }
        }
    }

    Ok(survivors)
}

/// Project a projection's `[[hooks]]` raw TOML payloads into typed entries.
/// Returns a human-readable reason string on the first malformed entry so the
/// caller can decide (per trust) between quarantine and whole-build failure.
pub(super) fn project_hook_entries(
    extension_id: &str,
    hooks: &[ironclaw_extensions::HookSectionEntryV2],
) -> Result<Vec<HookManifestEntry>, String> {
    let mut entries = Vec::with_capacity(hooks.len());
    for hook in hooks {
        let entry: HookManifestEntry = toml::from_str(&hook.raw_toml).map_err(|error| {
            format!(
                "extension `{extension_id}` hook `{}` is not a valid hook manifest entry: {error}",
                hook.local_id
            )
        })?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Build the per-run hook dispatcher builder factory for the production
/// runtime, or `None` when the framework is disabled.
///
/// - **Flag OFF** ⇒ returns `Ok(None)`. The runtime never composes a
///   dispatcher; behavior is identical to the pre-hooks runtime. This is the
///   default and the rollout-safety contract.
/// - **Flag ON** ⇒ projects + installs the first-party builtin hooks and every
///   admitted extension-declared hook into a *template* registry once, then
///   returns a closure that mints a fresh [`HookDispatcherBuilder`] per host
///   build by replaying the same surviving install set. The fresh-per-build
///   construction gives each run its own dispatcher (no cross-run poison /
///   counter leak), and the per-tenant `registry` + evaluator keep one tenant's
///   hooks isolated from another.
///
/// `registry` is a [`HookProjectionRegistry`] — hook-only metadata
/// ([`HookProjection`]) for the per-tenant extension set. It holds NO
/// `ExtensionRegistry` and NO `ExtensionPackage`, so the projected third-party
/// packages structurally cannot reach the capability-dispatch path: there is no
/// capability surface inside the type to leak (containment by data shape).
///
/// Trusted (builtin / host-bundled) packages fail the whole build on any
/// malformed hook (`?`); untrusted (installed/third-party) packages are
/// quarantined per-extension and projection continues. See
/// [`project_extension_hook_sets`].
pub fn build_hook_dispatcher_builder_factory(
    config: HooksActivationConfig,
    registry: &HookProjectionRegistry,
) -> Result<Option<HookDispatcherBuilderFactory>, RebornBuildError> {
    // Production path: the first-party catalog is empty
    // (`install_first_party_hooks` is a no-op). All other wiring lives in the
    // shared helper.
    build_hook_dispatcher_builder_factory_with(
        config,
        registry,
        // No tenant id/root threaded through the convenience entry point; the
        // projection registry has already passed admission caps + containment
        // in `build_hook_projection_registry`. A synthetic tenant label is used
        // only for any quarantine audit emitted during install-time validation.
        None,
        None,
        install_first_party_hooks,
    )
}

/// Tenant-attributed production entry point (serrrfirat's #3951 P1 finding #3).
///
/// Identical to [`build_hook_dispatcher_builder_factory`] except the
/// authenticated `tenant_id` (and its derived extension root) are threaded into
/// install-time quarantine audit attribution. The production composition root
/// ([`crate::runtime::build_reborn_runtime`]) holds the real `tenant_id`, so it
/// MUST use this path rather than the no-tenant convenience entry point — a
/// quarantine emitted while validating a tenant's hook install set is attributed
/// to that tenant, not the synthetic `"reborn-hook-projection"` fallback. This
/// closes the observability gap where discovery-time audits carried the real
/// tenant but install-time audits did not.
pub fn build_hook_dispatcher_builder_factory_for_tenant(
    config: HooksActivationConfig,
    registry: &HookProjectionRegistry,
    tenant_id: &ironclaw_host_api::TenantId,
) -> Result<Option<HookDispatcherBuilderFactory>, RebornBuildError> {
    // The tenant-derived extension root is the same root discovery/admission
    // computed; recomputing it here is deterministic (pure function of the
    // tenant id) and keeps the audit-context seam tenant-aware end to end.
    let root = tenant_extension_root(tenant_id)?;
    build_hook_dispatcher_builder_factory_with(
        config,
        registry,
        Some((tenant_id, &root)),
        None,
        install_first_party_hooks,
    )
}

pub(crate) fn build_hook_dispatcher_builder_factory_for_tenant_with_audit_sink(
    config: HooksActivationConfig,
    registry: &HookProjectionRegistry,
    tenant_id: &ironclaw_host_api::TenantId,
    audit_sink: Arc<dyn SecurityAuditSink>,
) -> Result<Option<HookDispatcherBuilderFactory>, RebornBuildError> {
    let root = tenant_extension_root(tenant_id)?;
    build_hook_dispatcher_builder_factory_with(
        config,
        registry,
        Some((tenant_id, &root)),
        Some(audit_sink),
        install_first_party_hooks,
    )
}

/// Shared implementation behind [`build_hook_dispatcher_builder_factory`],
/// parameterized on the first-party install step and an optional tenant context
/// (used for quarantine audit attribution during install-time validation).
///
/// `install_first_party` is invoked both at composition-time validation and on
/// every per-run builder mint, so it must be a pure replayable function of its
/// builder input. Production passes [`install_first_party_hooks`] (the empty
/// catalog); tests pass a closure that installs a test-only first-party hook,
/// exercising the activation machinery end-to-end through the real composition
/// path without shipping a production no-op.
pub(super) fn build_hook_dispatcher_builder_factory_with<F>(
    config: HooksActivationConfig,
    registry: &HookProjectionRegistry,
    tenant_context: Option<(
        &ironclaw_host_api::TenantId,
        &ironclaw_host_api::VirtualPath,
    )>,
    audit_sink: Option<Arc<dyn SecurityAuditSink>>,
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

    let registrar = HookRegistrar::new(Arc::clone(&evaluator));

    // Validate the first-party set up front against a scratch builder
    // (fail-closed). An empty install set is a legitimate state — a zero-binding
    // dispatcher composes fine.
    {
        let scratch = HookDispatcherBuilder::new(HookRegistry::new());
        let _validated = install_first_party(scratch)?;
    }

    // Project + validate the extension hook sets ONCE, applying atomic
    // per-extension quarantine for untrusted sources and fail-closed-whole-build
    // for trusted (builtin/host-bundled) sources. Survivors are replayed per
    // run. A fallback synthetic tenant label keeps audit events well-formed when
    // no explicit tenant context is threaded (the convenience entry point).
    let fallback_tenant =
        ironclaw_host_api::TenantId::new("reborn-hook-projection").map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("could not build fallback audit tenant id: {error}"),
            }
        })?;
    let (audit_tenant, audit_root): (
        &ironclaw_host_api::TenantId,
        Option<&ironclaw_host_api::VirtualPath>,
    ) = match tenant_context {
        Some((tenant, root)) => (tenant, Some(root)),
        None => (&fallback_tenant, None),
    };
    let extension_install_sets = project_extension_hook_sets(
        registry.projections(),
        &registrar,
        audit_tenant,
        audit_root,
        audit_sink.as_ref(),
    )?;

    let evaluator_for_factory = Arc::clone(&evaluator);
    let factory: HookDispatcherBuilderFactory = Arc::new(move || {
        // Fresh registry + builder per run: no cross-run state leak.
        let mut builder = HookDispatcherBuilder::new(HookRegistry::new());
        // `install_first_party` is a pure replayable function of the builder;
        // the identical call was proven to succeed against a scratch builder in
        // the composition-time validation block above (fail-closed via `?`). A
        // per-run replay therefore cannot fail in practice — the `?` propagates
        // any future regression as a build error rather than a panic (CLAUDE.md:
        // no `.expect()` in production code).
        builder = install_first_party(builder).map_err(|error| {
            reborn_replay_error(format!(
                "per-run first-party hook install replay failed \
                 (validated at composition time): {error}"
            ))
        })?;
        let registrar = HookRegistrar::new(Arc::clone(&evaluator_for_factory));
        for set in &extension_install_sets {
            // Each surviving set was already projected from TOML (the only
            // fallible external-input step) AND fully validated against a scratch
            // builder above (quarantined sets never reach here). The per-run
            // replay of a scratch-validated set cannot fail in practice; the `?`
            // surfaces any regression as a build error instead of a panic.
            let (next, _ids) = registrar
                .install(
                    set.extension_id.clone(),
                    &set.extension_version,
                    &set.entries,
                    builder,
                )
                .map_err(|error| {
                    reborn_replay_error(format!(
                        "per-run hook install replay failed for extension `{}` \
                         (validated at composition time): {error}",
                        set.extension_id.as_str()
                    ))
                })?;
            builder = next;
        }
        Ok(builder)
    });

    Ok(Some(factory))
}
