//! The bundled `ironclaw.memory` Extension Manifest v2 (issue #3537).
//!
//! This module owns the host-bundled native memory extension: its v2 TOML
//! manifest, the host service identity that backs it, and the function that
//! turns the bundled manifest into a registrable [`ExtensionPackage`].
//!
//! Native memory is loaded on the **always-on first-party lane** (like the
//! builtin toolset), not the catalog/lifecycle lane: [`native_memory_first_party_package`]
//! parses the bundled TOML and the composition layer inserts the resulting
//! package directly into the extension registry at startup. There is no
//! install/enable lifecycle. Co-locating the manifest with the service identity
//! embodies the issue's "bundled TOML alone is not authority" rule: the manifest
//! declares a `first_party` runtime whose `service` must match the host-registered
//! native memory provider ([`NATIVE_MEMORY_PROVIDER_SERVICE`]); the binding layer
//! (`memory_binding`) resolves that service to an `Arc<dyn MemoryService>`, and
//! the document-store profile binding is the provider-swap point.
//!
//! The four capabilities are model-visible memory tools. `read`/`write` implement
//! the `memory.document_store.v1` profile (their schema refs match the profile's
//! required-operation refs); `search`/`tree` are native conveniences. Input
//! schemas are served inline on the always-on lane (see
//! `first_party_tools::resolve_native_memory_input_schema_ref`), so no asset
//! materialization is required.

use ironclaw_extensions::{
    ExtensionError, ExtensionInstallationError, ExtensionManifestRecord, ExtensionManifestV2,
    ExtensionPackage, ManifestSource,
};
use ironclaw_host_api::VirtualPath;

use crate::extension_contracts::{default_host_api_contract_registry, default_host_port_catalog};

/// Reserved host-bundled extension id for the native memory provider.
pub const NATIVE_MEMORY_EXTENSION_ID: &str = "ironclaw.memory";

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

/// Raw bundled manifest TOML for the mem0 memory backend. Declarative provider
/// identity only (no tools of its own); the mem0 `MemoryService` is constructed
/// from `[memory]` config in composition, gated by the `memory-mem0` feature.
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

/// Build the registrable package for the bundled native memory extension.
///
/// Parses the bundled v3 TOML and converts it into an [`ExtensionPackage`]. The
/// composition layer inserts this package into the always-on extension registry
/// (alongside the builtin package), so native memory's model tools are
/// unconditionally available without a catalog install/enable step.
pub fn native_memory_first_party_package() -> Result<ExtensionPackage, ExtensionError> {
    let invalid = |error: &dyn std::fmt::Display| ExtensionError::InvalidManifest {
        reason: format!("native memory first-party package is invalid: {error}"),
    };
    let record =
        memory_manifest_record(NATIVE_MEMORY_MANIFEST_TOML).map_err(|error| invalid(&error))?;
    let manifest = record
        .manifest()
        .clone()
        .try_into()
        .map_err(|error: ExtensionError| invalid(&error))?;
    let root = VirtualPath::new(NATIVE_MEMORY_PACKAGE_ROOT)?;
    ExtensionPackage::from_manifest_toml(manifest, root, record.raw_toml())
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
    fn manifest_declares_four_model_visible_capabilities() {
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
        // Backend-only: mem0 declares the [memory] surface but no tools of its
        // own (the memory tool surface is the adapter's).
        assert!(record.manifest().capabilities.is_empty());
        let memory = record
            .resolved()
            .memory
            .as_ref()
            .expect("mem0 manifest declares the [memory] surface");
        assert!(memory.backs(ironclaw_host_api::MemoryOperationKind::DocumentStore));
    }
}
