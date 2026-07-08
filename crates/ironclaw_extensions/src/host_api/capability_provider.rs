use std::collections::BTreeSet;

use serde::Deserialize;

use crate::v2::{
    CapabilityDeclV2, HostApiId, HostApiManifestContext, HostApiManifestContract,
    HostApiManifestProjection, HostApiRefV2, HostApiSectionError, ManifestSectionPath,
    ManifestV2Error, RawCapabilityV2,
};

pub const CAPABILITY_PROVIDER_HOST_API_ID: &str = "ironclaw.capability_provider/v1";
pub const CAPABILITY_PROVIDER_SECTION: &str = "capability_provider.tools";

#[derive(Debug)]
pub struct CapabilityProviderHostApiContract {
    id: HostApiId,
}

impl CapabilityProviderHostApiContract {
    pub fn new() -> Result<Self, ManifestV2Error> {
        Ok(Self {
            id: HostApiId::new(CAPABILITY_PROVIDER_HOST_API_ID)?,
        })
    }
}

impl HostApiManifestContract for CapabilityProviderHostApiContract {
    fn id(&self) -> &HostApiId {
        &self.id
    }

    fn accepts_section_path(&self, section: &ManifestSectionPath) -> bool {
        section.as_str() == CAPABILITY_PROVIDER_SECTION
    }

    fn validate_section(
        &self,
        _host_api: &HostApiRefV2,
        _section: &toml::Value,
    ) -> Result<(), HostApiSectionError> {
        Err(HostApiSectionError::from(
            "capability provider validation requires manifest context",
        ))
    }

    fn validate_section_with_context(
        &self,
        context: &HostApiManifestContext<'_>,
        _host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), HostApiSectionError> {
        let _ = project_capabilities(context, section)?;
        Ok(())
    }

    fn project_section_with_context(
        &self,
        context: &HostApiManifestContext<'_>,
        _host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<HostApiManifestProjection, HostApiSectionError> {
        Ok(HostApiManifestProjection {
            capabilities: project_capabilities(context, section)?,
            surfaces: Vec::new(),
        })
    }
}

fn project_capabilities(
    context: &HostApiManifestContext<'_>,
    section: &toml::Value,
) -> Result<Vec<CapabilityDeclV2>, HostApiSectionError> {
    let parsed: CapabilityProviderToolsSection = section
        .clone()
        .try_into()
        .map_err(|error: toml::de::Error| HostApiSectionError::from(error.to_string()))?;
    if parsed.capabilities.is_empty() {
        return Err(HostApiSectionError::from(
            "capability_provider.tools must declare at least one capability",
        ));
    }

    let mut seen = BTreeSet::new();
    let mut capabilities = Vec::with_capacity(parsed.capabilities.len());
    for raw in parsed.capabilities {
        // `from_raw` errors keep their typed `ManifestV2Error` variants so
        // capability validation reports identically however the capability
        // is declared.
        let capability =
            CapabilityDeclV2::from_raw(raw, context.extension_id, context.host_port_catalog)?;
        if !seen.insert(capability.id.clone()) {
            return Err(ManifestV2Error::DuplicateCapability { id: capability.id }.into());
        }
        capabilities.push(capability);
    }
    Ok(capabilities)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityProviderToolsSection {
    #[serde(default)]
    capabilities: Vec<RawCapabilityV2>,
}
