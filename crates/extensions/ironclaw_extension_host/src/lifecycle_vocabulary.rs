use ironclaw_extension_registry::InstallationOwner;
use ironclaw_host_api::{
    action::NetworkTargetPattern,
    capability::{CapabilityDescriptor, EffectKind, PermissionMode, RuntimeCredentialRequirement},
    ids::{CapabilityId, ExtensionId},
};

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveExtensionCapability {
    pub id: CapabilityId,
    pub provider: ExtensionId,
    pub effects: Vec<EffectKind>,
    pub default_permission: PermissionMode,
    pub runtime_credentials: Vec<RuntimeCredentialRequirement>,
    /// Manifest-declared network egress allowlist, independent of credentials.
    pub network_targets: Vec<NetworkTargetPattern>,
    /// Manifest-declared per-capability egress cap in bytes. `None` means no cap.
    pub max_egress_bytes: Option<u64>,
    /// Owner of the providing extension installation.
    pub owner: InstallationOwner,
}

impl ActiveExtensionCapability {
    pub fn from_descriptor(descriptor: &CapabilityDescriptor, owner: InstallationOwner) -> Self {
        Self {
            id: descriptor.id.clone(),
            provider: descriptor.provider.clone(),
            effects: descriptor.effects.clone(),
            default_permission: descriptor.default_permission,
            runtime_credentials: descriptor.runtime_credentials.clone(),
            network_targets: descriptor.network_targets.clone(),
            max_egress_bytes: descriptor.max_egress_bytes,
            owner,
        }
    }
}
