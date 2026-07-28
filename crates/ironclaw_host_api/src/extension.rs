//! Neutral live-extension contract.
//!
//! Declarative manifest data remains owned by `ironclaw_extensions`; this
//! module defines the in-process callable shape a host publishes after loading
//! and binding an extension. The callable facets are the existing
//! [`ToolAdapter`] and [`ChannelAdapter`] contracts.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    CapabilityDescriptor, CapabilityId, ChannelAdapter, ChannelDescriptor, ExtensionId,
    HostApiError, ToolAdapter,
};

/// One loaded extension instance.
///
/// The value is stable for the lifetime of one binding. It is deliberately
/// separate from [`ExtensionId`]: multiple host-managed installations of the
/// same extension package may exist over time, and activation snapshots need a
/// stable instance identity for logs and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExtensionInstanceId(String);

impl ExtensionInstanceId {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_instance_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExtensionInstanceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for ExtensionInstanceId {
    type Error = HostApiError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ExtensionInstanceId> for String {
    fn from(value: ExtensionInstanceId) -> Self {
        value.0
    }
}

fn validate_instance_id(value: &str) -> Result<(), HostApiError> {
    let invalid = |reason: &str| HostApiError::InvalidId {
        kind: "extension_instance",
        value: value.to_string(),
        reason: reason.to_string(),
    };
    if value.is_empty() {
        return Err(invalid("must not be empty"));
    }
    if value.len() > 128 {
        return Err(invalid("must be at most 128 bytes"));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':' || c == '.')
    {
        return Err(invalid(
            "must contain only ASCII letters, digits, '-', '_', ':', or '.'",
        ));
    }
    Ok(())
}

/// Stable identity for one live extension binding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ExtensionRuntimeIdentity {
    pub extension_id: ExtensionId,
    pub instance_id: ExtensionInstanceId,
}

/// Host-visible declaration for one live extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionContract {
    pub identity: ExtensionRuntimeIdentity,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<CapabilityDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelDescriptor>,
}

impl ExtensionContract {
    pub fn capability(&self, capability_id: &crate::CapabilityId) -> Option<&CapabilityDescriptor> {
        self.capabilities
            .iter()
            .find(|descriptor| &descriptor.id == capability_id)
    }
}

/// One loaded extension with its callable facets.
///
/// Implementations must report metadata from the resolved host contract only.
/// Invocation goes through [`ToolAdapter`]; channel behavior goes through
/// [`ChannelAdapter`]. Authorization, approval, obligation, resource, and
/// ingress policy remain host-owned.
pub trait Extension: Send + Sync {
    fn contract(&self) -> &ExtensionContract;

    fn capability_adapter(&self) -> Option<Arc<dyn ToolAdapter>> {
        None
    }

    fn channel_adapter(&self) -> Option<Arc<dyn ChannelAdapter>> {
        None
    }
}

/// Generic extension-host assembly policy supplied by composition.
#[derive(Debug, Clone)]
pub struct ExtensionHostAssemblyConfig {
    pub reserved_capability_ids: BTreeSet<CapabilityId>,
    pub reserved_ingress_routes: BTreeSet<String>,
    pub hook_deadline: Duration,
}

impl ExtensionHostAssemblyConfig {
    pub fn new(
        reserved_capability_ids: BTreeSet<CapabilityId>,
        reserved_ingress_routes: BTreeSet<String>,
        hook_deadline: Duration,
    ) -> Self {
        Self {
            reserved_capability_ids,
            reserved_ingress_routes,
            hook_deadline,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_instance_id_accepts_installation_like_values() {
        let id = ExtensionInstanceId::new("slack:tenant-install_1").expect("valid id");
        assert_eq!(id.as_str(), "slack:tenant-install_1");
    }

    #[test]
    fn extension_instance_id_rejects_path_like_values() {
        let error = ExtensionInstanceId::new("../slack").expect_err("invalid id");
        assert!(error.to_string().contains("extension_instance"));
    }
}
