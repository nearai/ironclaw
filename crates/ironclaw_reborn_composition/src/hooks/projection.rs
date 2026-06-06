//! Hook-only projection: structural containment of the per-tenant extension set
//! into hook metadata that can reach ONLY the hook factory, plus the
//! discovery/admission pipeline that decides which third-party packages are
//! merged.
//!
//! This module owns the structural containment unit ([`HookProjection`]) and
//! the hook-only registry ([`HookProjectionRegistry`]) — the #3951 P1
//! least-privilege boundary: the projection path holds only `[[hooks]]`
//! payloads plus the identity/trust/root needed to install + contain them, and
//! NOTHING from the capability / runtime / package surface.

use ironclaw_extensions::ExtensionRegistry;

use crate::error::RebornBuildError;

use super::HooksActivationConfig;
use super::audit::emit_hook_quarantined;

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
pub(super) struct HookProjection {
    pub(super) extension_id: ironclaw_host_api::ExtensionId,
    pub(super) version: String,
    /// Trust posture (drives quarantine-vs-fail-closed). Copied off the
    /// manifest at extraction time; the capability surface is left behind.
    pub(super) source: ironclaw_extensions::ManifestSource,
    /// Package root, for the projection-layer containment check only.
    pub(super) root: ironclaw_host_api::VirtualPath,
    /// The declared `[[hooks]]` payloads — the ONLY package content carried.
    pub(super) hooks: Vec<ironclaw_extensions::HookSectionEntryV2>,
}

impl HookProjection {
    /// Extract the hook-only projection from an extension package, dropping
    /// everything else (capabilities, runtime, schema refs). Returns `None` for
    /// a package that declares no hooks (nothing to project).
    pub(super) fn from_package(package: &ironclaw_extensions::ExtensionPackage) -> Option<Self> {
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
    pub(super) fn from_projections(projections: Vec<HookProjection>) -> Self {
        Self(projections)
    }

    /// Crate-private read-only view of the projected hook metadata, for the
    /// hook projection loop only.
    pub(super) fn projections(&self) -> impl Iterator<Item = &HookProjection> {
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
pub(super) fn enforce_root_containment(
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
        // `VirtualPath` already canonicalizes `..`/`.`/empty (`//`) segments
        // out, but this is the explicit projection-layer no-`..`/no-empty-segment
        // guard the FS-hardening v1 posture documents — defense-in-depth that
        // does not rely on the path type's canonicalization (gemini #3951 #353).
        if segment == ".." || segment == "." || segment.is_empty() {
            return Err(format!(
                "package root `{candidate}` contains a traversal or empty segment `{segment}`"
            ));
        }
    }
    Ok(())
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
/// in [`super::factory::project_extension_hook_sets`]; this function applies the
/// *registry admission* caps + containment that decide which packages are even
/// merged.
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
