use serde::{Deserialize, Serialize};

use crate::{
    ApprovalRequest, CapabilityId, CapabilitySet, EffectKind, ExtensionId, MountView,
    ResourceEstimate, ScopedPath, SecretHandle,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretUseMode {
    InjectIntoRequest,
    InjectIntoEnvironment,
    ReadRaw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkScheme {
    Http,
    Https,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkTarget {
    pub scheme: NetworkScheme,
    pub host: String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkTargetPattern {
    pub scheme: Option<NetworkScheme>,
    pub host_pattern: String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub allowed_targets: Vec<NetworkTargetPattern>,
    pub deny_private_ip_ranges: bool,
    pub max_egress_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionLifecycleOperation {
    Install,
    Update,
    Remove,
    Enable,
    Disable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Action {
    ReadFile {
        path: ScopedPath,
    },
    ListDir {
        path: ScopedPath,
    },
    WriteFile {
        path: ScopedPath,
        bytes: Option<u64>,
    },
    DeleteFile {
        path: ScopedPath,
    },
    Dispatch {
        capability: CapabilityId,
        estimated_resources: ResourceEstimate,
    },
    Spawn {
        extension_id: ExtensionId,
        requested_capabilities: CapabilitySet,
        requested_mounts: MountView,
        estimated_resources: ResourceEstimate,
    },
    UseSecret {
        handle: SecretHandle,
        mode: SecretUseMode,
    },
    Network {
        target: NetworkTarget,
        method: NetworkMethod,
        estimated_bytes: Option<u64>,
    },
    ReserveResources {
        estimate: ResourceEstimate,
    },
    Approve {
        request: Box<ApprovalRequest>,
    },
    ExtensionLifecycle {
        extension_id: ExtensionId,
        operation: ExtensionLifecycleOperation,
    },
    EmitExternalEffect {
        effect: EffectKind,
    },
}
