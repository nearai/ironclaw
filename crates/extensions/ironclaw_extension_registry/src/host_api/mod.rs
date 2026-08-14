//! Built-in host API manifest contracts owned by `ironclaw_extension_registry`.

use std::sync::Arc;

use crate::v2::{HostApiContractRegistry, ManifestV2Error};

pub mod capability_provider;
pub mod product_adapter;

/// Build the default set of Extension Manifest v2 host API contracts: every
/// contract this module owns, in one [`HostApiContractRegistry`].
///
/// These contracts validate host-owned manifest declarations but do not execute
/// runtime code, resolve schema files, or publish hot surfaces — which is why
/// the default set lives beside the contracts it registers rather than in the
/// kernel that happened to be its first caller. Product-specific contracts are
/// added by the composition layer that owns those products.
pub fn default_host_api_contract_registry() -> Result<HostApiContractRegistry, ManifestV2Error> {
    let mut registry = HostApiContractRegistry::new();
    registry.register(Arc::new(
        capability_provider::CapabilityProviderHostApiContract::new()?,
    ))?;
    Ok(registry)
}
