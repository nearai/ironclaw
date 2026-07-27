//! The bundled memory provider manifests (issue #3537).
//!
//! This module owns the host-bundled memory provider extensions — their v3
//! TOML manifests, the host service identities that back them, and the
//! functions that turn each bundled manifest into a registrable
//! [`ExtensionPackage`] plus its declared lifecycle
//! ([`BundledMemoryProvider`]).
//!
//! The BOUND provider is loaded on the **always-on first-party lane** (like
//! the builtin toolset), not the catalog/lifecycle lane: composition resolves
//! the compose-time `[memory]` binding, loads THAT provider's bundle, and
//! inserts its package directly into the extension registry at startup. There
//! is no install/enable lifecycle. Co-locating the manifests with the service
//! identities embodies the "bundled TOML alone is not authority" rule: each
//! manifest declares a `first_party` runtime whose `service` must match the
//! host-registered provider identity; the binding layer (`memory_binding`)
//! decides which provider serves, and the manifest's `[[tools]]` +
//! `[memory].lifecycle` are the single source of truth for that provider's
//! surface.
//!
//! The declared tools are model-visible memory tools under the stable
//! `ironclaw.memory.*` ids. Input schemas are served inline on the always-on
//! lane (see `first_party_tools::resolve_native_memory_input_schema_ref`), so
//! no asset materialization is required.

use ironclaw_extensions::{
    ExtensionError, ExtensionInstallationError, ExtensionManifestRecord, ExtensionManifestV2,
    ExtensionPackage, ManifestSource,
};
use ironclaw_host_api::{MemoryDescriptor, VirtualPath};

use crate::extension_contracts::{default_host_api_contract_registry, default_host_port_catalog};

/// Reserved host-bundled extension id for the native memory provider.
pub const NATIVE_MEMORY_EXTENSION_ID: &str = "ironclaw.memory";

/// Every host-bundled memory provider package id. These are the only
/// extension ids the always-on memory lane can register a package under, so
/// identity checks that must hold for "the bound memory provider" (registry
/// provider allowlist, inline schema serving, first-party trust entries) key
/// off this list instead of hardwiring the native id.
pub const MEMORY_PROVIDER_PACKAGE_IDS: &[&str] =
    &[NATIVE_MEMORY_EXTENSION_ID, MEM0_MEMORY_EXTENSION_ID];

/// Host service identity declared by the manifest's `first_party` runtime. The
/// host must register a matching service for the bundled manifest to be
/// authoritative; this constant is the single source of truth both the manifest
/// (via a parse-time assertion test) and the binding layer compare against.
pub const NATIVE_MEMORY_PROVIDER_SERVICE: &str = "native_memory_provider";

/// Virtual package root for the bundled native memory extension. Used as a
/// stable identity for the registered package; on the always-on lane the
/// manifest's schemas are served inline rather than read from this path.
const NATIVE_MEMORY_PACKAGE_ROOT: &str = "/system/extensions/ironclaw.memory";

/// Raw bundled manifest TOML for the native memory extension.
pub const NATIVE_MEMORY_MANIFEST_TOML: &str = include_str!("../assets/memory_native/manifest.toml");

/// Reserved (host-bundled) extension id for the mem0 memory backend. Mirrors
/// `ironclaw_memory_mem0::MEM0_MEMORY_EXTENSION_ID`; the `[memory]` binding
/// selects it by this id.
pub const MEM0_MEMORY_EXTENSION_ID: &str = "mem0.local.memory";

/// Host service identity declared by the mem0 backend manifest's `first_party`
/// runtime.
pub const MEM0_MEMORY_PROVIDER_SERVICE: &str = "mem0_memory_provider";

/// Raw bundled manifest TOML for the mem0 memory backend: mem0's own tool
/// declarations (under the stable `ironclaw.memory.*` ids) plus its honest
/// lifecycle set. The mem0 `MemoryService` is constructed from `[memory]`
/// config in composition, gated by the `memory-mem0` feature.
pub const MEM0_MEMORY_MANIFEST_TOML: &str = include_str!("../assets/memory_mem0/manifest.toml");

/// Parse the bundled `ironclaw.memory` manifest into the internal manifest
/// model. Fail-closed: the reserved id, `first_party` runtime, `[memory]`
/// surface, schema refs, and provider-prefixed tool ids are validated by the
/// parser.
pub fn native_memory_manifest() -> Result<ExtensionManifestV2, ExtensionInstallationError> {
    Ok(memory_manifest_record(NATIVE_MEMORY_MANIFEST_TOML)?
        .manifest()
        .clone())
}

/// Parse + validate a bundled memory manifest into its resolved record.
/// `ExtensionManifestRecord::from_toml` is the single parse entry point; it
/// dispatches on `schema_version` (v2 or v3) and normalizes into one model.
fn memory_manifest_record(
    toml: &str,
) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
    let host_ports = default_host_port_catalog().map_err(|error| {
        ExtensionInstallationError::InvalidManifest {
            reason: error.to_string(),
        }
    })?;
    let contracts = default_host_api_contract_registry().map_err(|error| {
        ExtensionInstallationError::InvalidManifest {
            reason: error.to_string(),
        }
    })?;
    ExtensionManifestRecord::from_toml(
        toml,
        ManifestSource::HostBundled,
        &host_ports,
        None,
        &contracts,
    )
}

/// Virtual package root for the bundled mem0 memory backend.
const MEM0_MEMORY_PACKAGE_ROOT: &str = "/system/extensions/mem0.local.memory";

/// A bundled memory provider's registrable package plus the lifecycle hooks
/// its manifest declares — exactly what composition consumes when this
/// provider is the bound one: the package's declared tools are registered on
/// the always-on lane, and the lifecycle set gates every host-initiated
/// memory call.
pub struct BundledMemoryProvider {
    pub package: ExtensionPackage,
    pub lifecycle: MemoryDescriptor,
}

/// Build the registrable provider bundle for the bundled native memory
/// extension. The composition layer inserts the package into the always-on
/// extension registry (alongside the builtin package) when native is the
/// bound provider.
pub fn native_memory_provider_bundle() -> Result<BundledMemoryProvider, ExtensionError> {
    memory_provider_bundle(
        NATIVE_MEMORY_MANIFEST_TOML,
        NATIVE_MEMORY_PACKAGE_ROOT,
        "native memory",
    )
}

/// Build the registrable provider bundle for the bundled mem0 memory backend,
/// used when the compose-time `[memory]` binding selects mem0 AND the mem0
/// provider is actually constructible.
pub fn mem0_memory_provider_bundle() -> Result<BundledMemoryProvider, ExtensionError> {
    memory_provider_bundle(MEM0_MEMORY_MANIFEST_TOML, MEM0_MEMORY_PACKAGE_ROOT, "mem0")
}

/// Backward-compatible package-only accessor for the native provider bundle.
pub fn native_memory_first_party_package() -> Result<ExtensionPackage, ExtensionError> {
    Ok(native_memory_provider_bundle()?.package)
}

fn memory_provider_bundle(
    toml: &str,
    package_root: &str,
    label: &str,
) -> Result<BundledMemoryProvider, ExtensionError> {
    let invalid = |error: &dyn std::fmt::Display| ExtensionError::InvalidManifest {
        reason: format!("{label} memory provider package is invalid: {error}"),
    };
    let record = memory_manifest_record(toml).map_err(|error| invalid(&error))?;
    let lifecycle = record.resolved().memory.clone().unwrap_or_default();
    let manifest = record
        .manifest()
        .clone()
        .try_into()
        .map_err(|error: ExtensionError| invalid(&error))?;
    let root = VirtualPath::new(package_root)?;
    let package = ExtensionPackage::from_manifest_toml(manifest, root, record.raw_toml())?;
    Ok(BundledMemoryProvider { package, lifecycle })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID,
        MEMORY_WRITE_CAPABILITY_ID,
    };
    use ironclaw_extensions::{CapabilityVisibility, ExtensionRuntimeV2};

    #[test]
    fn manifest_parses_as_host_bundled_first_party() {
        let manifest = native_memory_manifest().expect("native memory manifest must parse");
        assert_eq!(manifest.id.as_str(), NATIVE_MEMORY_EXTENSION_ID);
        assert_eq!(manifest.source, ManifestSource::HostBundled);
        match &manifest.runtime {
            ExtensionRuntimeV2::FirstParty { service } => {
                // Bundled TOML alone is not authority: its declared service must
                // match the host-registered native memory provider identity.
                assert_eq!(service, NATIVE_MEMORY_PROVIDER_SERVICE);
            }
            other => panic!("expected first_party runtime, got {other:?}"),
        }
    }

    #[test]
    fn manifest_declares_the_model_visible_memory_tools() {
        let manifest = native_memory_manifest().expect("manifest");
        let ids: Vec<&str> = manifest
            .capabilities
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                MEMORY_READ_CAPABILITY_ID,
                MEMORY_WRITE_CAPABILITY_ID,
                MEMORY_SEARCH_CAPABILITY_ID,
                MEMORY_TREE_CAPABILITY_ID,
                crate::PROFILE_SET_CAPABILITY_ID,
            ]
        );
        for capability in &manifest.capabilities {
            assert_eq!(
                capability.visibility,
                CapabilityVisibility::Model,
                "{} must be model-visible",
                capability.id
            );
        }
    }

    #[test]
    fn native_memory_declares_no_host_ports() {
        // The live native provider is filesystem-backed; it declares no storage
        // or audit host ports. The SQL/audit ports remain catalogued vocabulary
        // for the deferred SQL-backed milestone (see ADR 0002), but no live
        // capability requires them.
        let manifest = native_memory_manifest().expect("manifest");
        for capability in &manifest.capabilities {
            assert!(
                capability.required_host_ports.is_empty(),
                "{} must declare no required host ports",
                capability.id
            );
        }
    }

    #[test]
    fn native_memory_package_builds() {
        let package = native_memory_first_party_package().expect("native memory package builds");
        assert_eq!(package.manifest.id.as_str(), NATIVE_MEMORY_EXTENSION_ID);
    }

    #[test]
    fn native_provider_bundle_declares_the_full_lifecycle() {
        use ironclaw_host_api::MemoryLifecycleHook;
        let bundle = native_memory_provider_bundle().expect("native bundle builds");
        assert_eq!(
            bundle.package.manifest.id.as_str(),
            NATIVE_MEMORY_EXTENSION_ID
        );
        for hook in MemoryLifecycleHook::ALL {
            assert!(
                bundle.lifecycle.declares(hook),
                "native must declare {hook:?}"
            );
        }
    }

    #[test]
    fn mem0_provider_bundle_builds_with_its_honest_lifecycle_and_tools() {
        use ironclaw_host_api::MemoryLifecycleHook;
        let bundle = mem0_memory_provider_bundle().expect("mem0 bundle builds");
        assert_eq!(
            bundle.package.manifest.id.as_str(),
            MEM0_MEMORY_EXTENSION_ID
        );
        assert!(bundle.lifecycle.declares(MemoryLifecycleHook::ReadLongTerm));
        assert!(bundle.lifecycle.declares(MemoryLifecycleHook::ProfileRead));
        assert!(
            !bundle
                .lifecycle
                .declares(MemoryLifecycleHook::ReadShortTerm)
        );
        assert!(
            !bundle
                .lifecycle
                .declares(MemoryLifecycleHook::RecordInteraction)
        );
        let ids: Vec<&str> = bundle
            .package
            .manifest
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                MEMORY_READ_CAPABILITY_ID,
                MEMORY_WRITE_CAPABILITY_ID,
                MEMORY_SEARCH_CAPABILITY_ID,
                MEMORY_TREE_CAPABILITY_ID,
                crate::PROFILE_SET_CAPABILITY_ID,
            ]
        );
    }

    #[test]
    fn mem0_backend_manifest_is_a_valid_v3_memory_provider() {
        let record = memory_manifest_record(MEM0_MEMORY_MANIFEST_TOML)
            .expect("mem0 backend manifest must parse");
        assert_eq!(record.manifest().id.as_str(), MEM0_MEMORY_EXTENSION_ID);
        match &record.manifest().runtime {
            ExtensionRuntimeV2::FirstParty { service } => {
                assert_eq!(service, MEM0_MEMORY_PROVIDER_SERVICE);
            }
            other => panic!("expected first_party runtime, got {other:?}"),
        }
        // mem0's manifest is the source of truth for its own tool surface:
        // the four document tools, declared under the reserved stable
        // `ironclaw.memory.*` ids so a backend swap never renames the
        // model's tools.
        let ids: Vec<&str> = record
            .manifest()
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                MEMORY_READ_CAPABILITY_ID,
                MEMORY_WRITE_CAPABILITY_ID,
                MEMORY_SEARCH_CAPABILITY_ID,
                MEMORY_TREE_CAPABILITY_ID,
                crate::PROFILE_SET_CAPABILITY_ID,
            ]
        );
        for capability in &record.manifest().capabilities {
            assert_eq!(capability.visibility, CapabilityVisibility::Model);
        }
        let memory = record
            .resolved()
            .memory
            .as_ref()
            .expect("mem0 manifest declares the [memory] surface");
        // The lifecycle declaration is honest (F5): mem0 implements the
        // long-term retrieval lane and profile reads, has no thread
        // partitioning (no short-term lane), and does not record
        // interactions — undeclared hooks are never called by the host.
        use ironclaw_host_api::MemoryLifecycleHook;
        assert!(memory.declares(MemoryLifecycleHook::ReadLongTerm));
        assert!(memory.declares(MemoryLifecycleHook::ProfileRead));
        assert!(!memory.declares(MemoryLifecycleHook::ReadShortTerm));
        assert!(!memory.declares(MemoryLifecycleHook::RecordInteraction));
    }
}
