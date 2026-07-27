use ironclaw_extensions::{HostApiContractRegistry, ManifestV2Error};

pub fn product_extension_host_api_contract_registry()
-> Result<HostApiContractRegistry, ManifestV2Error> {
    let mut registry = ironclaw_host_runtime::default_host_api_contract_registry()?;
    ironclaw_product::adapter_registry::register_product_adapter_host_api_contract(&mut registry)
        .map_err(|error| ManifestV2Error::Invalid {
        reason: format!("product adapter host API contract registration failed: {error}"),
    })?;
    Ok(registry)
}
