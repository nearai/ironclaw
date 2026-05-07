//! Action contracts for host authorization.
//!
//! An [`Action`] is the normalized description of something an execution wants
//! to do before any service performs it: read/write a scoped path, dispatch a
//! capability, spawn a capability-backed process, use a secret, contact the network, or
//! reserve resources. Runtime crates should convert their concrete operations
//! into these variants so policy, approvals, resources, and audit all reason
//! about the same shape. Actions intentionally contain scoped/virtual contract
//! types, never raw host paths or secret values.

use serde::{Deserialize, Serialize};

use crate::{
    ApprovalRequest, CapabilityId, EffectKind, ExtensionId, ResourceEstimate, ScopedPath,
    SecretHandle,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretUseMode {
    InjectIntoRequest,
    InjectIntoEnvironment,
    ReadRaw,
}

impl std::fmt::Display for SecretUseMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InjectIntoRequest => "inject_into_request",
            Self::InjectIntoEnvironment => "inject_into_environment",
            Self::ReadRaw => "read_raw",
        })
    }
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

impl std::fmt::Display for NetworkMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Put => "put",
            Self::Patch => "patch",
            Self::Delete => "delete",
            Self::Head => "head",
        })
    }
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

impl std::fmt::Display for ExtensionLifecycleOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::Remove => "remove",
            Self::Enable => "enable",
            Self::Disable => "disable",
        })
    }
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
    SpawnCapability {
        capability: CapabilityId,
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
