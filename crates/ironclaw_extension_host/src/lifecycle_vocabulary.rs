use std::sync::Arc;

use ironclaw_extensions::InstallationOwner;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, EffectKind, ExtensionId, NetworkTargetPattern,
    PermissionMode, ResourceScope, RuntimeCredentialRequirement, RuntimeHttpEgress,
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

#[derive(Clone)]
pub enum ExtensionActivationMode {
    Static,
    HostedMcpDiscovery {
        scope: ResourceScope,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    },
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

impl ExtensionActivationMode {
    pub fn from_dispatch_context(
        scope: ResourceScope,
        runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    ) -> Self {
        match runtime_http_egress {
            Some(runtime_http_egress) => Self::HostedMcpDiscovery {
                scope,
                runtime_http_egress,
            },
            None => Self::Static,
        }
    }
}
