//! The bundled `ironclaw.memory.native` Extension Manifest v2 (issue #3537).
//!
//! This module owns the host-bundled native memory extension manifest *and* the
//! host service identity that backs it. Co-locating the two embodies the issue's
//! "bundled TOML alone is not authority" rule: the manifest declares a
//! `first_party` runtime whose `service` must match the host-registered native
//! memory provider ([`NATIVE_MEMORY_PROVIDER_SERVICE`]); the binding layer
//! (`memory_binding`) is what actually resolves that service to an
//! `Arc<dyn MemoryService>`.
//!
//! The manifest is parsed with [`ManifestSource::HostBundled`] (the only source
//! eligible for a reserved `ironclaw.*` id and a `first_party` runtime) against
//! the host's [`default_host_port_catalog`](crate::default_host_port_catalog),
//! so the memory host ports must be registered there or parsing fails closed.
//!
//! The four provider-prefixed capabilities implement the three host-defined
//! memory profiles (see [`crate::memory_profiles`]). They are `host_internal`:
//! the host context/interaction pipeline invokes them, and the model-facing
//! surface stays the separate `builtin.memory_*` tools.

use ironclaw_extensions::{ExtensionManifestV2, ManifestSource, ManifestV2Error};

use crate::extension_contracts::default_host_port_catalog;

/// Reserved host-bundled extension id for the native memory provider.
pub const NATIVE_MEMORY_EXTENSION_ID: &str = "ironclaw.memory.native";

/// Host service identity declared by the manifest's `first_party` runtime. The
/// host must register a matching service for the bundled manifest to be
/// authoritative; this constant is the single source of truth both the manifest
/// (via a parse-time assertion test) and the binding layer compare against.
pub const NATIVE_MEMORY_PROVIDER_SERVICE: &str = "native_memory_provider";

/// Capability id implementing `memory.context_retrieval.v1`.
pub const NATIVE_MEMORY_CONTEXT_RETRIEVE_CAPABILITY_ID: &str =
    "ironclaw.memory.native.context.retrieve";
/// Capability id implementing `memory.interaction_log.v1`.
pub const NATIVE_MEMORY_INTERACTION_RECORD_CAPABILITY_ID: &str =
    "ironclaw.memory.native.interaction.record";
/// Capability id implementing the read operation of `memory.document_store.v1`.
pub const NATIVE_MEMORY_DOCUMENT_READ_CAPABILITY_ID: &str = "ironclaw.memory.native.document.read";
/// Capability id implementing the write operation of `memory.document_store.v1`.
pub const NATIVE_MEMORY_DOCUMENT_WRITE_CAPABILITY_ID: &str =
    "ironclaw.memory.native.document.write";

/// Raw bundled manifest TOML for the native memory extension.
pub const NATIVE_MEMORY_MANIFEST_TOML: &str = include_str!("../assets/memory_native/manifest.toml");

/// Parse and validate the bundled `ironclaw.memory.native` manifest.
///
/// Validation is fail-closed: the reserved id, `first_party` runtime, declared
/// host ports, schema refs, and provider-prefixed capability ids are all checked
/// against the host-bundled rules and the default host-port catalog.
pub fn native_memory_manifest() -> Result<ExtensionManifestV2, ManifestV2Error> {
    let catalog = default_host_port_catalog().map_err(ManifestV2Error::Contract)?;
    ExtensionManifestV2::parse(
        NATIVE_MEMORY_MANIFEST_TOML,
        ManifestSource::HostBundled,
        &catalog,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extensions::ExtensionRuntimeV2;

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
    fn manifest_declares_four_host_internal_capabilities() {
        use ironclaw_extensions::CapabilityVisibility;
        let manifest = native_memory_manifest().expect("manifest");
        let ids: Vec<&str> = manifest
            .capabilities
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                NATIVE_MEMORY_CONTEXT_RETRIEVE_CAPABILITY_ID,
                NATIVE_MEMORY_INTERACTION_RECORD_CAPABILITY_ID,
                NATIVE_MEMORY_DOCUMENT_READ_CAPABILITY_ID,
                NATIVE_MEMORY_DOCUMENT_WRITE_CAPABILITY_ID,
            ]
        );
        for capability in &manifest.capabilities {
            assert_eq!(
                capability.visibility,
                CapabilityVisibility::HostInternal,
                "{} must be host_internal",
                capability.id
            );
        }
    }

    #[test]
    fn every_capability_requires_the_memory_storage_and_audit_ports() {
        use ironclaw_host_api::{
            HOST_EVENTS_AUDIT_PORT_ID, HOST_STORAGE_SQL_TRANSACTION_FIRST_PARTY_PORT_ID, HostPortId,
        };
        let manifest = native_memory_manifest().expect("manifest");
        let storage = HostPortId::new(HOST_STORAGE_SQL_TRANSACTION_FIRST_PARTY_PORT_ID).unwrap();
        let audit = HostPortId::new(HOST_EVENTS_AUDIT_PORT_ID).unwrap();
        for capability in &manifest.capabilities {
            assert!(
                capability.required_host_ports.contains(&storage),
                "{} must require the sql_transaction storage port",
                capability.id
            );
            assert!(
                capability.required_host_ports.contains(&audit),
                "{} must require the audit port",
                capability.id
            );
        }
    }
}
