use ironclaw_extension_registry::{HostApiContractRegistry, ManifestV2Error};

pub fn product_extension_host_api_contract_registry()
-> Result<HostApiContractRegistry, ManifestV2Error> {
    let mut registry = ironclaw_extension_registry::default_host_api_contract_registry()?;
    ironclaw_extension_registry::host_api::product_adapter::register_product_adapter_host_api_contract(
        &mut registry,
    )
    .map_err(|error| ManifestV2Error::Invalid {
        reason: format!("product adapter host API contract registration failed: {error}"),
    })?;
    Ok(registry)
}
