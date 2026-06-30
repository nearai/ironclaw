use std::sync::Arc;

use ironclaw_extensions::{
    CapabilityProviderHostApiContract, ExtensionDiscovery, ExtensionError, ExtensionRegistry,
    HostApiContractRegistry, ManifestV2Error, TolerantBoundedDiscovery,
};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    HOST_RUNTIME_HTTP_EGRESS_PORT_ID, HostApiError, HostPortCatalog, HostPortCatalogEntry,
    HostPortId, VirtualPath,
};
use ironclaw_product_adapter_registry::ProductAdapterHostApiContract;

/// Build the host-runtime default set of Extension Manifest v2 host API contracts.
///
/// This is composition-only: contracts validate and project manifest declarations,
/// but do not execute runtime code, resolve schema files, or publish hot surfaces.
pub fn default_host_api_contract_registry() -> Result<HostApiContractRegistry, ManifestV2Error> {
    let mut registry = HostApiContractRegistry::new();
    registry.register(Arc::new(CapabilityProviderHostApiContract::new()?))?;
    let product_adapter_contract =
        ProductAdapterHostApiContract::new().map_err(|error| ManifestV2Error::Invalid {
            reason: format!("product adapter host API contract registration failed: {error}"),
        })?;
    registry.register(Arc::new(product_adapter_contract))?;
    Ok(registry)
}

/// Build the host-runtime default host-port validation catalog.
///
/// The catalog is validation vocabulary only. It does not grant authority or
/// construct the concrete runtime HTTP egress adapter.
pub fn default_host_port_catalog() -> Result<HostPortCatalog, HostApiError> {
    HostPortCatalog::new(vec![HostPortCatalogEntry::new(HostPortId::new(
        HOST_RUNTIME_HTTP_EGRESS_PORT_ID,
    )?)])
}

/// Discover installed extensions through host-runtime's default host API
/// contracts and default host-port validation catalog.
pub async fn discover_extensions_with_default_host_api_contracts<F>(
    fs: &F,
    root: &VirtualPath,
) -> Result<ExtensionRegistry, ExtensionError>
where
    F: RootFilesystem,
{
    let host_port_catalog = default_host_port_catalog()?;
    discover_extensions_with_default_host_api_contracts_and_catalog(fs, root, &host_port_catalog)
        .await
}

/// Discover installed extensions through host-runtime's default host API
/// contracts and caller-supplied host-port validation catalog.
pub async fn discover_extensions_with_default_host_api_contracts_and_catalog<F>(
    fs: &F,
    root: &VirtualPath,
    host_port_catalog: &HostPortCatalog,
) -> Result<ExtensionRegistry, ExtensionError>
where
    F: RootFilesystem,
{
    let contracts = default_host_api_contract_registry()?;
    ExtensionDiscovery::discover_with_manifest_contracts(
        fs,
        root,
        ironclaw_extensions::ManifestSource::InstalledLocal,
        host_port_catalog,
        &contracts,
    )
    .await
}

/// Tolerant + bounded discovery through host-runtime's default contracts.
///
/// Wraps [`ExtensionDiscovery::discover_with_manifest_contracts_tolerant_bounded`]
/// with the default host API contracts + port catalog. Bounds the read/parse
/// work to `max_extensions` directory entries and quarantines per-package
/// failures instead of aborting the whole discovery; only failure to LIST the
/// root surfaces as the outer `Err`. The hook-projection composition path uses
/// this so a single malformed third-party manifest (or thousands of extension
/// directories) cannot drop or DoS the rest of a tenant's hook set.
pub async fn discover_extensions_tolerant_bounded<F>(
    fs: &F,
    root: &VirtualPath,
    max_extensions: usize,
) -> Result<TolerantBoundedDiscovery, ExtensionError>
where
    F: RootFilesystem,
{
    let host_port_catalog = default_host_port_catalog()?;
    let contracts = default_host_api_contract_registry()?;
    ExtensionDiscovery::discover_with_manifest_contracts_tolerant_bounded(
        fs,
        root,
        ironclaw_extensions::ManifestSource::InstalledLocal,
        &host_port_catalog,
        &contracts,
        max_extensions,
    )
    .await
}
