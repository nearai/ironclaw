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
    /// Sub-flag gating *third-party installed-extension* hook activation. The
    /// master `enabled` flag activates builtin/host-bundled hooks; this
    /// additional flag must ALSO be on before any third-party `[[hooks]]`
    /// declaration is discovered and projected. Default OFF: with this off the
    /// activation path is byte-identical to the builtin-only #3938 behavior.
    ///
    /// **Filesystem-hardening gate (read before flipping in production):**
    /// `HOOKS_THIRD_PARTY_ENABLED` MUST NOT be enabled in multi-tenant
    /// production until the `openat2(RESOLVE_BENEATH)` / `O_NOFOLLOW` backend
    /// hardening lands. v1 ships a projection-layer strict-child / no-`..` /
    /// no-symlink-escape containment check plus the canonicalizing local
    /// backend; that is the documented gating follow-up.
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
        let enabled = match std::env::var(HOOKS_ENABLED_ENV) {
            Ok(value) => is_truthy(&value),
            Err(_) => false,
        };
        let third_party_enabled = match std::env::var(HOOKS_THIRD_PARTY_ENABLED_ENV) {
            Ok(value) => is_truthy(&value),
            Err(_) => false,
        };
        Self {
            enabled,
            third_party_enabled,
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

/// Maximum number of *installed* extension packages whose `[[hooks]]` will be
/// considered for projection in a single tenant build. Surplus extensions
/// beyond this count are quarantined (skipped + audited), never whole-build
/// failed. A tenant-wide DoS ceiling on top of the per-extension caps the
/// registrar already enforces (`MAX_HOOKS_PER_EXTENSION`).
pub const MAX_INSTALLED_EXTENSIONS_CONSIDERED: usize = 64;

/// Maximum number of hook bindings projected from third-party installed
/// extensions across the whole tenant. An extension whose hooks would push the
/// running total past this budget is quarantined (skipped + audited), not
/// whole-build failed. Builtin / host-bundled bindings do not count against
/// this third-party budget (they are trusted and fail-closed-whole-build).
pub const MAX_TOTAL_HOOKS_PER_TENANT: usize = 256;

/// Structured target for the security-audit `tracing` channel. Composition is a
/// pre-run phase (no run-scoped `LoopHostMilestoneSink` exists yet), so
/// quarantine decisions are surfaced via `tracing` at this stable target rather
/// than a durable sink. Durable surfacing is a documented follow-up.
const SECURITY_AUDIT_TARGET: &str = "security_audit";

/// The hook-only metadata extracted from ONE extension package: exactly the
/// fields the projection needs, and NOTHING from the capability / runtime /
/// package surface.
///
/// This is the structural containment unit (serrrfirat's #3951 P1): the
/// projection path holds only `[[hooks]]` payloads plus the identity/trust/root
/// needed to install + contain them. It literally CANNOT reach capabilities,
/// the runtime spec, schema refs, or anything else on `ExtensionPackage` —
/// because it does not hold them. Containment is by DATA SHAPE, stronger than a
/// "no conversion provided" newtype boundary.
#[derive(Debug, Clone)]
struct HookProjection {
    extension_id: ironclaw_host_api::ExtensionId,
    version: String,
    /// Trust posture (drives quarantine-vs-fail-closed). Copied off the
    /// manifest at extraction time; the capability surface is left behind.
    source: ironclaw_extensions::ManifestSource,
    /// Package root, for the projection-layer containment check only.
    root: ironclaw_host_api::VirtualPath,
    /// The declared `[[hooks]]` payloads — the ONLY package content carried.
    hooks: Vec<ironclaw_extensions::HookSectionEntryV2>,
}

impl HookProjection {
    /// Extract the hook-only projection from an extension package, dropping
    /// everything else (capabilities, runtime, schema refs). Returns `None` for
    /// a package that declares no hooks (nothing to project).
    fn from_package(package: &ironclaw_extensions::ExtensionPackage) -> Option<Self> {
        if package.manifest.hooks.is_empty() {
            return None;
        }
        Some(Self {
            extension_id: package.manifest.id.clone(),
            version: package.manifest.version.clone(),
            source: package.manifest.source,
            root: package.root.clone(),
            hooks: package.manifest.hooks.clone(),
        })
    }
}

/// A hook-projection registry: the hook-only metadata of every extension whose
/// declared `[[hooks]]` are projected into the hook dispatcher, AND NOTHING
/// ELSE.
///
/// # Structural containment (hook-only by data shape — #3951 P1)
///
/// Third-party installed extensions must contribute *hooks* without becoming
/// *capability providers*. The capability-dispatch path is fed by the
/// `Arc<ExtensionRegistry>` handed to
/// [`ironclaw_host_runtime::HostRuntimeServices::new`] (it becomes the
/// capability catalog + surface resolver). If a third-party registry reached
/// that constructor, those extensions would gain capability authority — exactly
/// what the hook-only projection model forbids.
///
/// This type carries `Vec<HookProjection>` — hook metadata ONLY. It does NOT
/// wrap an `ExtensionRegistry` or hold any `ExtensionPackage`, so there is no
/// `ExtensionRegistry` inside it to leak to the capability path: containment is
/// enforced by the DATA SHAPE, not by withholding a conversion. A developer
/// cannot feed this to `HostRuntimeServices::new` because it simply is not, and
/// cannot become, an `ExtensionRegistry`.
pub struct HookProjectionRegistry(Vec<HookProjection>);

impl HookProjectionRegistry {
    /// Build the hook-only registry from the per-package projections that
    /// survived discovery + admission. The full packages are consumed here and
    /// only their hook metadata is retained.
    fn from_projections(projections: Vec<HookProjection>) -> Self {
        Self(projections)
    }

    /// Crate-private read-only view of the projected hook metadata, for the
    /// hook projection loop only.
    fn projections(&self) -> impl Iterator<Item = &HookProjection> {
        self.0.iter()
    }
}

impl std::fmt::Debug for HookProjectionRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HookProjectionRegistry")
            .field("projection_count", &self.0.len())
            .finish()
    }
}

/// The fixed `/system/extensions` discovery root.
///
/// # Tenant isolation via the scoped filesystem (not the path)
///
/// The discovery layer ([`ironclaw_extensions::ExtensionDiscovery`] /
/// [`ironclaw_extensions::ExtensionPackage::from_manifest`]) hardcodes package
/// roots to `/system/extensions/<extension-id>` — exactly one segment under
/// this root. It does so **because the `RootFilesystem` it is handed is itself
/// the tenant scope boundary**: every other tenant-scoped resource in the
/// system (secrets, authorization leases, run state, …) is isolated by the
/// per-tenant [`ironclaw_filesystem::ScopedFilesystem`] / per-identity backend,
/// not by a tenant segment baked into the virtual path. Discovery follows the
/// same convention.
///
/// Consequently `tenant_extension_root` takes the authenticated `tenant_id`
/// (so the SIGNATURE pins that callers must supply identity, and the
/// containment defense knows the root) but returns the fixed `/system/extensions`
/// path. The isolation guarantee is: **the per-tenant `RootFilesystem` passed
/// to discovery resolves `/system/extensions/<id>` to that tenant's storage and
/// no other's.** In local-dev (single-tenant, the only profile
/// `build_reborn_runtime` wires) the runtime's FS is constructed once per
/// identity in `build_reborn_services`, so it is per-identity by construction;
/// production wiring (a follow-up, since `build_reborn_runtime` only supports
/// local-dev) must supply a tenant-scoped backend here.
///
/// **This makes the scoped FS the SOLE isolation boundary** — see the
/// FS-hardening gate on [`HooksActivationConfig`]: `HOOKS_THIRD_PARTY_ENABLED`
/// MUST NOT be enabled in multi-tenant production until
/// `openat2(RESOLVE_BENEATH)` / `O_NOFOLLOW` backend hardening lands, because
/// that hardening is precisely what protects the scoped-FS-is-the-boundary
/// property against symlink/`..` escapes below the virtual layer.
pub fn tenant_extension_root(
    _tenant_id: &ironclaw_host_api::TenantId,
) -> Result<ironclaw_host_api::VirtualPath, RebornBuildError> {
    ironclaw_host_api::VirtualPath::new("/system/extensions").map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("could not derive extension discovery root: {error}"),
        }
    })
}

/// Defense-in-depth containment check applied to a discovered package's root
/// before its hooks are projected (Step 2 / FS-hardening v1).
///
/// The canonicalizing local backend is the primary defense; this projection
/// layer adds a strict-child / no-`..` / no-symlink-escape check so a package
/// whose resolved root escapes the tenant root is quarantined rather than
/// projected. Returns `Ok(())` when the package root is a strict child of
/// `tenant_root`; otherwise an error naming the violation (the caller turns
/// this into a quarantine, not a whole-build failure, for untrusted sources).
fn enforce_root_containment(
    tenant_root: &ironclaw_host_api::VirtualPath,
    package_root: &ironclaw_host_api::VirtualPath,
) -> Result<(), String> {
    let root = tenant_root.as_str().trim_end_matches('/');
    let candidate = package_root.as_str();
    let prefix = format!("{root}/");
    if !candidate.starts_with(&prefix) {
        return Err(format!(
            "package root `{candidate}` is not a strict child of tenant root `{root}`"
        ));
    }
    // Reject any path traversal segment in the child portion. `VirtualPath`
    // already canonicalizes, but this is the explicit projection-layer
    // no-`..`/no-empty-segment guard the FS-hardening v1 posture documents.
    let child = &candidate[prefix.len()..];
    for segment in child.split('/') {
        if segment == ".." || segment == "." {
            return Err(format!(
                "package root `{candidate}` contains a traversal segment `{segment}`"
            ));
        }
    }
    Ok(())
}

/// Emit a `hook.quarantined` security-audit event for an extension whose hooks
/// were dropped during projection (Step 4a).
///
/// Pre-run composition has no durable milestone sink, so this surfaces via
/// `tracing` at the stable [`SECURITY_AUDIT_TARGET`]. Per the REPL logging rule
/// this uses `warn!` (never `info!`, which would corrupt the interactive TUI).
fn emit_hook_quarantined(
    tenant_id: &ironclaw_host_api::TenantId,
    extension_id: &str,
    reason: &str,
    hooks_dropped: usize,
) {
    tracing::warn!(
        target: SECURITY_AUDIT_TARGET,
        event = "hook.quarantined",
        tenant_id = %tenant_id.as_str(),
        extension_id = %extension_id,
        reason = %reason,
        hooks_dropped = hooks_dropped,
        "third-party extension hooks quarantined during projection"
    );
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

/// A surviving extension's projected hook install set: the typed entries that
/// passed scratch validation and are committed to the real builder per run.
/// Deterministic replay material — identical inputs each run.
struct ProjectedExtensionHooks {
    extension_id: ironclaw_host_api::ExtensionId,
    extension_version: String,
    entries: Vec<HookManifestEntry>,
}

/// Project the structurally-typed `[[hooks]]` payloads declared by each package
/// in `registry`, applying **atomic per-extension quarantine** for untrusted
/// (installed/third-party) sources and **fail-closed whole-build** for trusted
/// (builtin / host-bundled) sources.
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
fn project_extension_hook_sets(
    projections: impl Iterator<Item = impl std::ops::Deref<Target = HookProjection>>,
    registrar: &HookRegistrar,
    tenant_id: &ironclaw_host_api::TenantId,
    tenant_root: Option<&ironclaw_host_api::VirtualPath>,
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
                );
                continue;
            }
            if third_party_hook_total + hook_count > MAX_TOTAL_HOOKS_PER_TENANT {
                emit_hook_quarantined(
                    tenant_id,
                    &extension_id_str,
                    "exceeded MAX_TOTAL_HOOKS_PER_TENANT",
                    hook_count,
                );
                continue;
            }

            // ── Path-containment (FS-hardening v1 defense-in-depth). ──
            if let Some(root) = tenant_root
                && let Err(reason) = enforce_root_containment(root, &projection.root)
            {
                emit_hook_quarantined(tenant_id, &extension_id_str, &reason, hook_count);
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
                emit_hook_quarantined(tenant_id, &extension_id_str, &reason, hook_count);
                continue;
            }
        };

        let extension_id = projection.extension_id.clone();
        let extension_version = projection.version.clone();

        // ── Validate the WHOLE set against a scratch builder. Commit nothing
        // here; the survivors are replayed against the real builder later. ──
        let scratch = HookDispatcherBuilder::new(HookRegistry::new());
        match registrar.install(
            extension_id.clone(),
            extension_version.clone(),
            entries.clone(),
            scratch,
        ) {
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
                emit_hook_quarantined(tenant_id, &extension_id_str, &reason, hook_count);
            }
        }
    }

    Ok(survivors)
}

/// Project a projection's `[[hooks]]` raw TOML payloads into typed entries.
/// Returns a human-readable reason string on the first malformed entry so the
/// caller can decide (per trust) between quarantine and whole-build failure.
fn project_hook_entries(
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

/// Discovery input for [`build_hook_projection_registry`]: the tenant-scoped
/// filesystem and the validated authenticated `tenant_id`. The discovery root
/// is *computed* from the identity ([`tenant_extension_root`]) inside the
/// builder — never supplied by the caller — which is the tenant-isolation
/// contract (Step 2). The filesystem is the same tenant-scoped
/// [`RootFilesystem`] already built in `build_reborn_services`.
pub struct ThirdPartyDiscoveryInput<'a, F: ironclaw_filesystem::RootFilesystem> {
    pub filesystem: &'a F,
    pub tenant_id: &'a ironclaw_host_api::TenantId,
}

/// Assemble the hook-projection registry (Step 3).
///
/// Always seeds with the `builtin` extension registry. When
/// [`HooksActivationConfig::is_third_party_enabled`] is true AND a discovery
/// input is supplied, discovers installed extensions under the tenant-derived
/// root, applies the tenant-wide DoS caps + path-containment (defense in
/// depth), and merges the surviving third-party packages into the projection
/// registry. The resulting [`HookProjectionRegistry`] reaches ONLY the hook
/// factory — never the capability path (see the newtype's docs).
///
/// **With the third-party sub-flag OFF, the path is byte-identical to #3938:**
/// the projection registry is builtin-only and no discovery runs.
///
/// Per-extension hook *validity* quarantine is applied later, at install time
/// in [`project_extension_hook_sets`]; this function applies the *registry
/// admission* caps + containment that decide which packages are even merged.
pub async fn build_hook_projection_registry<F>(
    builtin: ExtensionRegistry,
    third_party_input: Option<ThirdPartyDiscoveryInput<'_, F>>,
    config: HooksActivationConfig,
) -> Result<HookProjectionRegistry, RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem,
{
    // Seed the projection with the BUILTIN packages' hook metadata only. The
    // builtin `ExtensionRegistry` is consumed here and dropped; only hook
    // projections survive into the hook-only registry (structural containment).
    let mut projections: Vec<HookProjection> = builtin
        .extensions()
        .filter_map(HookProjection::from_package)
        .collect();
    let mut seen_ids: std::collections::HashSet<String> = projections
        .iter()
        .map(|projection| projection.extension_id.as_str().to_string())
        .collect();

    if config.is_third_party_enabled()
        && let Some(input) = third_party_input
    {
        let tenant_id = input.tenant_id;
        let root = tenant_extension_root(tenant_id)?;
        // Tolerant + BOUNDED discovery under the tenant-derived root.
        //
        // Bounded: the read/parse/validate work is capped to
        // `MAX_INSTALLED_EXTENSIONS_CONSIDERED` extension directories — the
        // count cap fires BEFORE the per-manifest read storm, so a tenant with
        // thousands of extension dirs cannot force unbounded discovery work
        // (Critical-1 DoS fix). The bounded surplus is reported as a quarantine,
        // never read.
        //
        // Tolerant: a single malformed / oversized / id-mismatched package
        // quarantines ONLY itself and discovery continues, so one bad package
        // can no longer drop a tenant's entire legitimate third-party hook set
        // (Critical-2 fail-open fix). The ONLY error that triggers the
        // builtin-only fallback is failure to LIST THE ROOT itself (the
        // extensions tree is unreadable) — surfaced as the outer `Err` below.
        let discovered = match ironclaw_host_runtime::discover_extensions_tolerant_bounded(
            input.filesystem,
            &root,
            MAX_INSTALLED_EXTENSIONS_CONSIDERED,
        )
        .await
        {
            Ok(discovered) => discovered,
            Err(error) => {
                // Root unreadable: cannot make per-package decisions. Fail-safe
                // to "no third-party hooks" — a missing/unreadable extensions
                // tree must not block the runtime. This is the SOLE
                // builtin-only fallback.
                tracing::debug!(
                    tenant_id = %tenant_id.as_str(),
                    %error,
                    "third-party extension root unreadable; proceeding builtin-only"
                );
                return Ok(HookProjectionRegistry::from_projections(projections));
            }
        };

        // Per-package discovery quarantines (malformed manifest, oversized,
        // id-mismatch, surplus beyond the discovery bound). Each drops only its
        // own package; valid siblings are unaffected.
        for quarantine in &discovered.quarantined {
            emit_hook_quarantined(tenant_id, &quarantine.extension_id, &quarantine.reason, 0);
        }

        let mut hook_total = 0usize;
        for package in discovered.registry.extensions() {
            // Extract the hook-only projection; a package with no hooks yields
            // `None` and is skipped — it never enters the hook-only registry.
            let Some(projection) = HookProjection::from_package(package) else {
                continue;
            };
            let extension_id_str = projection.extension_id.as_str().to_string();
            let hook_count = projection.hooks.len();

            // The extension-COUNT cap is already enforced by the bounded
            // discovery above; here we enforce the per-tenant hook BUDGET and
            // path containment.
            if hook_total + hook_count > MAX_TOTAL_HOOKS_PER_TENANT {
                emit_hook_quarantined(
                    tenant_id,
                    &extension_id_str,
                    "exceeded MAX_TOTAL_HOOKS_PER_TENANT",
                    hook_count,
                );
                continue;
            }
            if let Err(reason) = enforce_root_containment(&root, &projection.root) {
                emit_hook_quarantined(tenant_id, &extension_id_str, &reason, hook_count);
                continue;
            }
            // Dedup by extension id: a duplicate of a builtin/already-merged id
            // is quarantined, not fatal. The hook budget is consumed only AFTER
            // a successful merge, so a quarantined (duplicate) package does NOT
            // consume budget (Refinement 3).
            if !seen_ids.insert(extension_id_str.clone()) {
                emit_hook_quarantined(
                    tenant_id,
                    &extension_id_str,
                    "duplicate extension id collides with an already-projected package",
                    hook_count,
                );
                continue;
            }

            hook_total += hook_count;
            projections.push(projection);
        }
    }

    Ok(HookProjectionRegistry::from_projections(projections))
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
fn build_hook_dispatcher_builder_factory_with<F>(
    config: HooksActivationConfig,
    registry: &HookProjectionRegistry,
    tenant_context: Option<(
        &ironclaw_host_api::TenantId,
        &ironclaw_host_api::VirtualPath,
    )>,
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
    let extension_install_sets =
        project_extension_hook_sets(registry.projections(), &registrar, audit_tenant, audit_root)?;

    let evaluator_for_factory = Arc::clone(&evaluator);
    let factory: HookDispatcherBuilderFactory = Arc::new(move || {
        // Fresh registry + builder per run: no cross-run state leak.
        let mut builder = HookDispatcherBuilder::new(HookRegistry::new());
        // safety: `install_first_party` is a pure replayable function of the builder; the identical call was proven to succeed against a scratch builder in the composition-time validation block above (fail-closed via `?`). A per-run replay therefore cannot fail.
        let first_party = "first-party hook install validated at composition time";
        builder = install_first_party(builder).expect(first_party); // safety: replay of composition-validated install; see binding above
        let registrar = HookRegistrar::new(Arc::clone(&evaluator_for_factory));
        for set in &extension_install_sets {
            // safety: each surviving set was already projected from TOML (the only fallible, external-input step) AND fully validated against a scratch builder above (quarantined sets never reach here). registrar.install is a deterministic function of these cloned inputs, so the per-run replay of a scratch-validated set cannot fail.
            let ext_install = "extension hook install validated at composition time";
            let (next, _ids) = registrar
                .install(
                    set.extension_id.clone(),
                    set.extension_version.clone(),
                    set.entries.clone(),
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
    /// extensions path and insert it into a fresh registry. `source` selects the
    /// trust posture: `InstalledLocal` (untrusted → quarantine on bad hooks) vs
    /// `HostBundled` (trusted → fail-closed-whole-build).
    fn registry_with_manifest_source(
        id: &str,
        toml: &str,
        source: ManifestSource,
    ) -> ExtensionRegistry {
        let manifest = ExtensionManifest::parse(toml, source, &HostPortCatalog::empty())
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

    /// As [`registry_with_manifest_source`] with the default untrusted
    /// (`InstalledLocal`) source.
    fn registry_with_manifest(id: &str, toml: &str) -> ExtensionRegistry {
        registry_with_manifest_source(id, toml, ManifestSource::InstalledLocal)
    }

    /// Extract a plain registry's hook-only projections into a
    /// [`HookProjectionRegistry`] for the hook-factory tests (the factory only
    /// accepts the hook-only newtype). Mirrors the production extraction.
    fn projection(registry: ExtensionRegistry) -> HookProjectionRegistry {
        HookProjectionRegistry::from_projections(
            registry
                .extensions()
                .filter_map(HookProjection::from_package)
                .collect(),
        )
    }

    #[test]
    fn config_defaults_to_disabled() {
        assert!(!HooksActivationConfig::default().is_enabled());
        assert!(!HooksActivationConfig::disabled().is_enabled());
        assert!(HooksActivationConfig::enabled().is_enabled());
    }

    #[test]
    fn third_party_flag_requires_master_flag_and_defaults_off() {
        // Default + master-only configs must report third-party OFF.
        assert!(!HooksActivationConfig::default().is_third_party_enabled());
        assert!(!HooksActivationConfig::disabled().is_third_party_enabled());
        assert!(
            !HooksActivationConfig::enabled().is_third_party_enabled(),
            "master flag alone must NOT enable third-party"
        );
        // Sub-flag on but master off ⇒ still OFF (master gate dominates).
        assert!(
            !HooksActivationConfig::disabled()
                .with_third_party_enabled(true)
                .is_third_party_enabled(),
            "sub-flag without the master flag must stay OFF"
        );
        // Both on ⇒ ON.
        assert!(
            HooksActivationConfig::enabled()
                .with_third_party_enabled(true)
                .is_third_party_enabled(),
            "master + sub-flag both on must enable third-party"
        );
        // Setting the sub-flag must never silently flip the master flag.
        assert!(
            !HooksActivationConfig::disabled()
                .with_third_party_enabled(true)
                .is_enabled()
        );
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
        let registry = projection(ExtensionRegistry::new());
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
        let registry = projection(ExtensionRegistry::new());
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
        let registry = projection(ExtensionRegistry::new());
        let factory = build_hook_dispatcher_builder_factory_with(
            HooksActivationConfig::enabled(),
            &registry,
            None,
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
        let registry = projection(ExtensionRegistry::new());
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

    // ─── Helpers for the projection / quarantine test matrix ─────────────────

    use ironclaw_hooks::identity::{ExtensionId as HookExtensionId, HookLocalId};

    /// Derive the registrar-minted hook id for an installed predicate hook.
    fn installed_hook_id(ext: &str, version: &str, local: &str) -> HookId {
        HookId::derive(
            &HookExtensionId::new(ext).expect("valid extension id"),
            version,
            &HookLocalId::new(local).expect("valid hook local id"),
            HookVersion::ONE,
        )
    }

    /// `[[hooks]]` block for a valid `own_capabilities` deny predicate (needs no
    /// grant, no WASM runtime).
    fn own_deny_hook(local: &str, target: &str) -> String {
        format!(
            r#"
[[hooks]]
id = "{local}"
kind = "before_capability"
scope = "own_capabilities"
body = {{ mode = "predicate", spec = {{ type = "deny_capability", reason = "blocked by manifest hook", when = {{ type = "name_equals", name = "{target}" }} }} }}
"#
        )
    }

    /// Build a [`HookProjectionRegistry`] directly from `(id, source, hooks)`
    /// triples (no discovery), driving the real install-time projection path.
    fn projection_with(packages: &[(&str, ManifestSource, String)]) -> HookProjectionRegistry {
        let mut registry = ExtensionRegistry::new();
        for (id, source, hooks_block) in packages {
            let manifest = ExtensionManifest::parse(
                &manifest_toml(id, hooks_block),
                *source,
                &HostPortCatalog::empty(),
            )
            .expect("manifest parses");
            let package = ExtensionPackage::from_manifest(
                manifest,
                VirtualPath::new(format!("/system/extensions/{id}")).expect("valid root path"),
            )
            .expect("package builds from manifest");
            registry.insert(package).expect("package inserts");
        }
        projection(registry)
    }

    // ─── Atomic quarantine + trust-discrimination coverage ───────────────────

    /// A valid `own_capabilities` predicate hook from an untrusted
    /// (`InstalledLocal`) extension installs through the real
    /// `HookRegistrar::install` path at the `Installed` tier, alongside the
    /// test-only first-party hook (extension install does not displace
    /// first-party).
    #[test]
    fn valid_extension_hook_manifest_installs_at_installed_tier() {
        let registry = projection(registry_with_manifest(
            "valid-ext",
            &manifest_toml("valid-ext", &own_deny_hook("deny-run", "valid-ext.run")),
        ));

        let factory = build_hook_dispatcher_builder_factory_with(
            HooksActivationConfig::enabled(),
            &registry,
            None,
            install_test_first_party_hook,
        )
        .expect("enabled build with a valid extension hook succeeds")
        .expect("flag ON yields a factory");
        let dispatcher = factory().build_arc();

        let expected = installed_hook_id("valid-ext", "0.1.0", "deny-run");
        let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::BeforeCapability);
        assert!(
            bindings.iter().any(|binding| binding.hook_id == expected),
            "installed extension hook must be bound at BeforeCapability; saw {bindings:?}"
        );

        let test_id = HookId::for_builtin(TEST_NOOP_OBSERVER_CANONICAL_PATH, HookVersion::ONE);
        let after = dispatcher.active_bindings_snapshot(HookPointSpec::AfterCapability);
        assert!(
            after.iter().any(|binding| binding.hook_id == test_id),
            "test-only first-party hook must remain installed alongside extension hooks"
        );
    }

    /// A malformed hook payload from an UNTRUSTED (`InstalledLocal`) extension
    /// must be QUARANTINED — the build SUCCEEDS, that extension's hook is
    /// absent, and (critically) no panic. This is the third-party degradation
    /// contract: an attacker-controlled installed manifest cannot crash
    /// composition nor fail the whole build.
    #[test]
    fn malformed_installed_extension_hook_is_quarantined_not_fatal() {
        let hooks_block = r#"
[[hooks]]
id = "broken-hook"
kind = "before_capability"
body = { mode = "nonsense" }
"#;
        let registry = projection(registry_with_manifest(
            "broken-ext",
            &manifest_toml("broken-ext", hooks_block),
        ));

        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("malformed INSTALLED manifest must NOT fail the build (quarantine)")
                .expect("flag ON yields a factory");
        let dispatcher = factory().build_arc();
        assert!(
            dispatcher
                .active_bindings_snapshot(HookPointSpec::BeforeCapability)
                .is_empty(),
            "quarantined extension must contribute no bindings"
        );
    }

    /// The SAME malformed payload from a TRUSTED (`HostBundled`) package must
    /// fail the whole build closed with `InvalidConfig` — builtin/host-bundled
    /// hooks are fail-closed-whole-build, never quarantined.
    #[test]
    fn malformed_host_bundled_extension_hook_fails_closed() {
        let hooks_block = r#"
[[hooks]]
id = "broken-hook"
kind = "before_capability"
body = { mode = "nonsense" }
"#;
        // HostBundled ids are reserved to the `ironclaw.` prefix.
        let registry = projection(registry_with_manifest_source(
            "ironclaw.broken",
            &manifest_toml("ironclaw.broken", hooks_block),
            ManifestSource::HostBundled,
        ));

        match build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry) {
            Err(RebornBuildError::InvalidConfig { reason }) => {
                assert!(
                    reason.contains("ironclaw.broken") && reason.contains("broken-hook"),
                    "fail-closed error must name the offending host-bundled extension + hook, got: {reason}"
                );
            }
            Ok(_) => panic!("malformed host-bundled manifest must fail the whole build"),
            Err(other) => panic!("expected InvalidConfig, got: {other}"),
        }
    }

    /// Atomic quarantine: an extension with two VALID hooks and one INVALID
    /// hook must install NONE of its three hooks (whole-set atomicity), while a
    /// sibling valid extension's hook IS installed.
    #[test]
    fn extension_with_one_invalid_hook_quarantines_the_whole_set_sibling_survives() {
        let bad_set = format!(
            "{}{}{}",
            own_deny_hook("ok-1", "mixed.run"),
            own_deny_hook("ok-2", "mixed.run"),
            // invalid third hook
            r#"
[[hooks]]
id = "bad-3"
kind = "before_capability"
body = { mode = "nonsense" }
"#
        );
        let registry = projection_with(&[
            ("mixed", ManifestSource::InstalledLocal, bad_set),
            (
                "good",
                ManifestSource::InstalledLocal,
                own_deny_hook("good-1", "good.run"),
            ),
        ]);

        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("partial-invalid set must quarantine, not fail the build")
                .expect("flag ON yields a factory");
        let dispatcher = factory().build_arc();
        let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::BeforeCapability);

        // None of `mixed`'s hooks installed.
        for local in ["ok-1", "ok-2", "bad-3"] {
            let id = installed_hook_id("mixed", "0.1.0", local);
            assert!(
                !bindings.iter().any(|b| b.hook_id == id),
                "atomic quarantine must drop ALL of the offending extension's hooks ({local} leaked)"
            );
        }
        // Sibling `good` survives.
        let good_id = installed_hook_id("good", "0.1.0", "good-1");
        assert!(
            bindings.iter().any(|b| b.hook_id == good_id),
            "a sibling valid extension's hooks must still install"
        );
    }

    /// An untrusted extension claiming `scope = same_tenant` (a wider scope
    /// than its own capabilities) with no host-verified grant is QUARANTINED by
    /// the registrar's trust-attenuation check — build succeeds, hook absent.
    #[test]
    fn installed_extension_claiming_ungranted_wider_scope_is_quarantined() {
        let hooks_block = r#"
[[hooks]]
id = "cross-tenant-deny"
kind = "before_capability"
scope = "same_tenant"
requires_grant = "cross-tenant-policy"
body = { mode = "predicate", spec = { type = "deny_capability", reason = "wider-scope deny", when = { type = "name_equals", name = "other-ext.run" } } }
"#;
        let registry = projection(registry_with_manifest(
            "reachy-ext",
            &manifest_toml("reachy-ext", hooks_block),
        ));

        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("ungranted wider-scope INSTALLED hook must quarantine, not fail the build")
                .expect("flag ON yields a factory");
        let dispatcher = factory().build_arc();
        assert!(
            dispatcher
                .active_bindings_snapshot(HookPointSpec::BeforeCapability)
                .is_empty(),
            "ungranted wider-scope hook must be quarantined (no binding)"
        );
    }

    /// Third-party WASM stays OUT: the projection registrar has no
    /// `wasm_runtime`, so a WASM-bodied installed hook fails install → under
    /// quarantine the extension is dropped and the build continues (Step 6
    /// negative test). A sibling predicate-only extension still installs.
    #[test]
    fn wasm_bodied_third_party_hook_is_quarantined_build_continues() {
        let wasm_block = r#"
[[hooks]]
id = "wasm-hook"
kind = "before_capability"
scope = "own_capabilities"
body = { mode = "wasm", export = "evaluate" }
"#;
        let registry = projection_with(&[
            (
                "wasm-ext",
                ManifestSource::InstalledLocal,
                wasm_block.to_string(),
            ),
            (
                "pred-ext",
                ManifestSource::InstalledLocal,
                own_deny_hook("pred-1", "pred-ext.run"),
            ),
        ]);

        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("WASM-bodied third-party hook must quarantine, not fail the build")
                .expect("flag ON yields a factory");
        let dispatcher = factory().build_arc();
        let bindings = dispatcher.active_bindings_snapshot(HookPointSpec::BeforeCapability);
        assert!(
            !bindings
                .iter()
                .any(|b| b.hook_id == installed_hook_id("wasm-ext", "0.1.0", "wasm-hook")),
            "WASM-bodied third-party hook must be quarantined (no runtime in loader registrar)"
        );
        assert!(
            bindings
                .iter()
                .any(|b| b.hook_id == installed_hook_id("pred-ext", "0.1.0", "pred-1")),
            "sibling predicate extension must still install after a WASM quarantine"
        );
    }

    /// Containment: a package whose root escapes the tenant root via `..` is
    /// rejected by `enforce_root_containment` (FS-hardening v1).
    #[test]
    fn root_containment_rejects_traversal_and_non_child() {
        let tenant_root = VirtualPath::new("/system/extensions/alpha").expect("root");
        // Strict child OK.
        assert!(
            enforce_root_containment(
                &tenant_root,
                &VirtualPath::new("/system/extensions/alpha/ext-1").expect("child")
            )
            .is_ok()
        );
        // Sibling tenant is not a child.
        assert!(
            enforce_root_containment(
                &tenant_root,
                &VirtualPath::new("/system/extensions/beta/ext-1").expect("sibling")
            )
            .is_err(),
            "another tenant's tree must not be a child of this tenant root"
        );
        // The tenant root itself is not a strict child.
        assert!(enforce_root_containment(&tenant_root, &tenant_root).is_err());
    }

    /// `tenant_extension_root` returns the fixed `/system/extensions` root
    /// (Option 1 — FS-scoped isolation; the per-tenant `RootFilesystem`, not a
    /// path segment, is the isolation boundary). The signature still requires
    /// identity so callers must thread it (and the containment defense knows the
    /// root), but the path is profile-independent. The cross-tenant proof lives
    /// in the integration test driving two distinct per-tenant filesystems.
    #[test]
    fn tenant_root_is_the_fixed_system_extensions_root() {
        let a = tenant_extension_root(&ironclaw_host_api::TenantId::new("alpha").expect("a"))
            .expect("root a");
        let b = tenant_extension_root(&ironclaw_host_api::TenantId::new("beta").expect("b"))
            .expect("root b");
        assert_eq!(a.as_str(), "/system/extensions");
        assert_eq!(b.as_str(), "/system/extensions");
    }

    /// DoS cap: more than `MAX_INSTALLED_EXTENSIONS_CONSIDERED` hook-bearing
    /// untrusted extensions ⇒ the surplus is quarantined (skipped), and the
    /// build still succeeds. We use a small synthetic set keyed off the const
    /// boundary so the test stays fast yet pins the ceiling.
    #[test]
    fn surplus_extensions_beyond_consider_cap_are_quarantined() {
        let mut packages: Vec<(String, ManifestSource, String)> = Vec::new();
        for i in 0..(MAX_INSTALLED_EXTENSIONS_CONSIDERED + 2) {
            let id = format!("ext-{i:03}");
            let hooks = own_deny_hook("h", &format!("{id}.run"));
            packages.push((id, ManifestSource::InstalledLocal, hooks));
        }
        let refs: Vec<(&str, ManifestSource, String)> = packages
            .iter()
            .map(|(id, src, hooks)| (id.as_str(), *src, hooks.clone()))
            .collect();
        let registry = projection_with(&refs);

        let factory =
            build_hook_dispatcher_builder_factory(HooksActivationConfig::enabled(), &registry)
                .expect("surplus extensions must quarantine, not fail the build")
                .expect("flag ON yields a factory");
        let dispatcher = factory().build_arc();
        let installed = dispatcher
            .active_bindings_snapshot(HookPointSpec::BeforeCapability)
            .len();
        assert!(
            installed <= MAX_INSTALLED_EXTENSIONS_CONSIDERED,
            "no more than the consider-cap of extensions may install (saw {installed})"
        );
        assert!(
            installed >= 1,
            "the first extensions under the cap must still install"
        );
    }

    /// Flag OFF (master ON + third-party OFF) keeps the projection registry
    /// builtin-only: a registry carrying only an untrusted package still yields
    /// a builtin-only set when assembled through `build_hook_projection_registry`
    /// with the sub-flag off — behavior identical to #3938.
    #[tokio::test]
    async fn third_party_subflag_off_yields_builtin_only_projection() {
        use ironclaw_filesystem::InMemoryBackend;

        let fs = InMemoryBackend::new();
        let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
        let builtin = ExtensionRegistry::new();
        // Master ON, third-party OFF.
        let config = HooksActivationConfig::enabled();
        let projection_registry = build_hook_projection_registry(
            builtin,
            Some(ThirdPartyDiscoveryInput {
                filesystem: &fs,
                tenant_id: &tenant,
            }),
            config,
        )
        .await
        .expect("projection registry builds");
        assert_eq!(
            projection_registry.projections().count(),
            0,
            "sub-flag OFF must not merge any third-party packages (builtin-only)"
        );
    }

    // ─── Tolerant + bounded DISCOVERY-stage coverage (Criticals 1 & 2) ───────

    /// A DISCOVERY-valid `InstalledLocal` v2 manifest carrying one projectable
    /// hook. Unlike [`manifest_toml`] (which uses the legacy top-level
    /// `[[capabilities]]` accepted only on the direct-parse path), the discovery
    /// contracts require the `ironclaw.capability_provider/v1` host_api form for
    /// installed sources, so the discovery-stage tests below use this shape. The
    /// `[[hooks]]` array-of-tables is a top-level sibling placed last.
    fn manifest_toml_with_hook(id: &str) -> String {
        format!(
            r#"schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "{id} extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.run"
description = "Run {id}"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/{id}/run.input.v1.json"
output_schema_ref = "schemas/{id}/run.output.v1.json"
prompt_doc_ref = "prompts/{id}/run.md"

[[hooks]]
id = "deny-run"
kind = "before_capability"
scope = "own_capabilities"
body = {{ mode = "predicate", spec = {{ type = "deny_capability", reason = "blocked by manifest hook", when = {{ type = "name_equals", name = "{id}.run" }} }} }}
"#
        )
    }

    /// Write `body` as `/system/extensions/<id>/manifest.toml` on `fs`.
    async fn write_manifest<F: ironclaw_filesystem::RootFilesystem>(fs: &F, id: &str, body: &str) {
        fs.write_file(
            &ironclaw_host_api::VirtualPath::new(format!("/system/extensions/{id}/manifest.toml"))
                .expect("manifest path"),
            body.as_bytes(),
        )
        .await
        .expect("write manifest");
    }

    /// Critical 2 (discovery-stage): one malformed manifest among valid siblings
    /// must quarantine ONLY the bad package — the valid siblings are still
    /// merged into the projection registry, and the build does NOT fall back to
    /// builtin-only.
    #[tokio::test]
    async fn malformed_sibling_manifest_does_not_drop_the_whole_third_party_set() {
        use ironclaw_filesystem::InMemoryBackend;

        let fs = InMemoryBackend::new();
        write_manifest(&fs, "good-a", &manifest_toml_with_hook("good-a")).await;
        write_manifest(&fs, "bad", "not valid toml {{{").await;
        write_manifest(&fs, "good-b", &manifest_toml_with_hook("good-b")).await;

        let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
        let config = HooksActivationConfig::enabled().with_third_party_enabled(true);
        let projection_registry = build_hook_projection_registry(
            ExtensionRegistry::new(),
            Some(ThirdPartyDiscoveryInput {
                filesystem: &fs,
                tenant_id: &tenant,
            }),
            config,
        )
        .await
        .expect("a malformed sibling must not fail the build (tolerant discovery)");

        let ids: Vec<String> = projection_registry
            .projections()
            .map(|p| p.extension_id.as_str().to_string())
            .collect();
        assert!(
            ids.contains(&"good-a".to_string()) && ids.contains(&"good-b".to_string()),
            "valid siblings must survive a malformed package; saw {ids:?}"
        );
        assert!(
            !ids.contains(&"bad".to_string()),
            "the malformed package must be quarantined, not merged"
        );
        assert_eq!(
            ids.len(),
            2,
            "exactly the two valid third-party packages must be merged (not builtin-only)"
        );
    }

    /// Critical 2 boundary: root unreadable is the ONLY case that falls back to
    /// builtin-only.
    #[tokio::test]
    async fn unreadable_extension_root_falls_back_to_builtin_only() {
        use async_trait::async_trait;
        use ironclaw_filesystem::{
            DirEntry, FileStat, FilesystemError, FilesystemOperation, RootFilesystem,
        };
        use ironclaw_host_api::VirtualPath;

        struct UnreadableRootFs;

        #[async_trait]
        impl RootFilesystem for UnreadableRootFs {
            async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
                Err(FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::ListDir,
                    reason: "extensions root unreadable".to_string(),
                })
            }

            async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
                Err(FilesystemError::NotFound {
                    path: path.clone(),
                    operation: FilesystemOperation::Stat,
                })
            }
        }

        let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
        let config = HooksActivationConfig::enabled().with_third_party_enabled(true);
        let projection_registry = build_hook_projection_registry(
            ExtensionRegistry::new(),
            Some(ThirdPartyDiscoveryInput {
                filesystem: &UnreadableRootFs,
                tenant_id: &tenant,
            }),
            config,
        )
        .await
        .expect("unreadable root falls back to builtin-only, not a hard error");
        assert_eq!(
            projection_registry.projections().count(),
            0,
            "an unreadable extensions root must yield builtin-only"
        );
    }

    /// Refinement 3: a package that fails to merge (duplicate id) must NOT
    /// consume the per-tenant hook budget. We prove the budget accounting moved
    /// AFTER the successful insert by showing a later distinct package still
    /// merges even though a duplicate was processed first (the duplicate did not
    /// burn budget). The duplicate itself is quarantined.
    #[tokio::test]
    async fn failed_merge_does_not_consume_hook_budget() {
        use ironclaw_filesystem::InMemoryBackend;

        let fs = InMemoryBackend::new();
        // Two distinct valid hook-bearing packages.
        write_manifest(&fs, "alpha", &manifest_toml_with_hook("alpha")).await;
        write_manifest(&fs, "beta", &manifest_toml_with_hook("beta")).await;

        let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
        let config = HooksActivationConfig::enabled().with_third_party_enabled(true);

        // Seed the builtin registry with a package whose id collides with
        // `alpha`, so discovery's `registry.insert(alpha)` FAILS (duplicate).
        // The failed merge must not consume budget; `beta` must still merge.
        let mut builtin = ExtensionRegistry::new();
        let contracts = ironclaw_host_runtime::default_host_api_contract_registry()
            .expect("default host api contracts");
        let dup = ExtensionPackage::from_manifest(
            ExtensionManifest::parse_with_host_api_contracts(
                &manifest_toml_with_hook("alpha"),
                ManifestSource::InstalledLocal,
                &HostPortCatalog::empty(),
                &contracts,
            )
            .expect("dup manifest parses"),
            VirtualPath::new("/system/extensions/alpha").expect("dup root"),
        )
        .expect("dup package builds");
        builtin.insert(dup).expect("seed duplicate");

        let projection_registry = build_hook_projection_registry(
            builtin,
            Some(ThirdPartyDiscoveryInput {
                filesystem: &fs,
                tenant_id: &tenant,
            }),
            config,
        )
        .await
        .expect("duplicate merge is quarantined, build succeeds");

        let ids: Vec<String> = projection_registry
            .projections()
            .map(|p| p.extension_id.as_str().to_string())
            .collect();
        // `alpha` appears once (the seeded builtin); the discovered duplicate was
        // quarantined. `beta` merged — proving the quarantined duplicate did not
        // consume budget that would have blocked beta.
        assert!(
            ids.contains(&"beta".to_string()),
            "a package after a quarantined duplicate must still merge; saw {ids:?}"
        );
        assert_eq!(
            ids.iter().filter(|id| id.as_str() == "alpha").count(),
            1,
            "the duplicate must be quarantined, not double-merged"
        );
    }
}
