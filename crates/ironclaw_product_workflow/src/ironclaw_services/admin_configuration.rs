//! Product DTOs for the manifest-declared administrator configuration view.

use serde::{Deserialize, Serialize};

use super::{IronClawViewDescriptor, ProductCapabilityDescriptor};

pub const ADMIN_CONFIGURATION_VIEW: IronClawViewDescriptor = IronClawViewDescriptor {
    id: "admin_configuration",
    paginated: false,
};
pub const ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID: &str = "builtin.admin_configuration_replace";
pub const ADMIN_CONFIGURATION_REPLACE_CAPABILITY: ProductCapabilityDescriptor =
    ProductCapabilityDescriptor::api_only(ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAdminConfigurationListResponse {
    pub groups: Vec<IronClawAdminConfigurationGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAdminConfigurationGroup {
    pub group_id: String,
    pub display_name: String,
    pub description: String,
    pub revision: u64,
    pub complete: bool,
    pub fields: Vec<IronClawAdminConfigurationField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used_by: Vec<IronClawAdminConfigurationUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAdminConfigurationField {
    pub handle: String,
    pub label: String,
    pub secret: bool,
    pub required: bool,
    pub provided: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IronClawAdminConfigurationUse {
    pub package_id: String,
    pub display_name: String,
    pub installed: bool,
}
