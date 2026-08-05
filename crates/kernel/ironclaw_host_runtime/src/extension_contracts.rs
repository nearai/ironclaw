//! Host-runtime extension discovery over the default manifest contracts.
//!
//! The two *default sets* this module used to define now live with the
//! vocabulary they enumerate — `ironclaw_host_api::host_port::
//! default_host_port_catalog` and `ironclaw_extension_registry::
//! default_host_api_contract_registry` (WS3 row 3, PROPOSAL §6.5.9). What stays
//! here is the discovery that binds them to a `RootFilesystem`, which is
//! host-runtime's own job.

use ironclaw_extension_registry::{
    ExtensionDiscovery, ExtensionError, ExtensionRegistry, HostApiContractRegistry,
    TolerantBoundedDiscovery, default_host_api_contract_registry,
};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    host_port::{HostPortCatalog, default_host_port_catalog},
    path::VirtualPath,
};

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
        ironclaw_extension_registry::ManifestSource::InstalledLocal,
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
    let contracts = default_host_api_contract_registry()?;
    discover_extensions_tolerant_bounded_with_contracts(fs, root, &contracts, max_extensions).await
}

/// Tolerant + bounded discovery through caller-supplied host API contracts.
///
/// The host-port catalog remains host-owned, while composition layers can add
/// product contracts without teaching host runtime about those products.
pub async fn discover_extensions_tolerant_bounded_with_contracts<F>(
    fs: &F,
    root: &VirtualPath,
    contracts: &HostApiContractRegistry,
    max_extensions: usize,
) -> Result<TolerantBoundedDiscovery, ExtensionError>
where
    F: RootFilesystem,
{
    let host_port_catalog = default_host_port_catalog()?;
    ExtensionDiscovery::discover_with_manifest_contracts_tolerant_bounded(
        fs,
        root,
        ironclaw_extension_registry::ManifestSource::InstalledLocal,
        &host_port_catalog,
        contracts,
        max_extensions,
    )
    .await
}
