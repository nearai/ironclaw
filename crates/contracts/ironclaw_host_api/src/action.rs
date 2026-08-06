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
    approval::ApprovalRequest,
    capability::EffectKind,
    ids::{CapabilityId, ExtensionId, SecretHandle},
    path::ScopedPath,
    resource::ResourceEstimate,
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

impl NetworkTargetPattern {
    /// Deliberately more permissive than `ironclaw_network::parse_host_pattern`
    /// (`crates/ironclaw_network/src/policy.rs`), which rejects a bare `*`.
    /// That sibling is the chokepoint for untrusted operator-typed hostnames
    /// (e.g. `IRONCLAW_SANDBOX_EXTRA_ALLOWED_DOMAINS`) that were never
    /// reviewed; this one validates already-reviewed declarations —
    /// host-authored `NetworkPolicy` construction — where a bare `*`
    /// legitimately means "every host", e.g.
    /// `CapabilityNetworkProfile::DevWildcard`'s local-dev shell profile
    /// (`ironclaw_composition::builtin_capability_policy::dev_wildcard_network_policy`).
    /// The two validators diverge INTENTIONALLY on this point: do not
    /// tighten this one to match `parse_host_pattern`'s wildcard rejection —
    /// doing so breaks `DevWildcard`'s declared full-access grant. (Extension
    /// manifest credential audiences are stricter still and already reject a
    /// wildcard host at parse time — `ManifestV3Error::WildcardAudienceHost`
    /// in `crates/ironclaw_extension_registry/src/v3.rs` — so they are not an example
    /// of something this permissiveness exists for.)
    pub fn validate_declaration(&self) -> Result<(), crate::error::HostApiError> {
        validate_network_host_pattern(&self.host_pattern)?;
        Ok(())
    }
}

fn validate_network_host_pattern(pattern: &str) -> Result<(), crate::error::HostApiError> {
    if pattern.is_empty() || pattern.len() > 253 {
        return Err(crate::error::HostApiError::invalid_network_target(
            pattern,
            "host pattern must be non-empty and at most 253 bytes",
        ));
    }
    if pattern == "*" {
        return Ok(());
    }
    if pattern.contains('\0') || pattern.chars().any(char::is_control) {
        return Err(crate::error::HostApiError::invalid_network_target(
            pattern,
            "host pattern must not contain NUL/control characters",
        ));
    }
    let host = pattern.strip_prefix("*.").unwrap_or(pattern);
    if host.is_empty()
        || host.starts_with('.')
        || host.ends_with('.')
        || host.contains("..")
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(crate::error::HostApiError::invalid_network_target(
            pattern,
            "host pattern must contain only ASCII host-label characters",
        ));
    }
    Ok(())
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
