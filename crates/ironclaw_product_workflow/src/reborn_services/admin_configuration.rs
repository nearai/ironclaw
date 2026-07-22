//! Product DTOs for the manifest-declared administrator configuration view.

use serde::{Deserialize, Serialize};

use super::RebornViewDescriptor;

pub const ADMIN_CONFIGURATION_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "admin_configuration",
    paginated: false,
};
pub const ADMIN_CONFIGURATION_REPLACE_CAPABILITY_ID: &str = "builtin.admin_configuration_replace";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAdminConfigurationListResponse {
    pub groups: Vec<RebornAdminConfigurationGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAdminConfigurationGroup {
    pub group_id: String,
    pub display_name: String,
    pub description: String,
    pub revision: u64,
    pub complete: bool,
    pub fields: Vec<RebornAdminConfigurationField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used_by: Vec<RebornAdminConfigurationUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAdminConfigurationField {
    pub handle: String,
    pub label: String,
    pub secret: bool,
    pub required: bool,
    pub provided: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornAdminConfigurationUse {
    pub package_id: String,
    pub display_name: String,
    pub installed: bool,
}
