//! Action contracts for host authorization.
//!
//! An [`Action`] is the normalized description of something an execution wants
//! to do before any service performs it: read/write a scoped path, dispatch a
//! capability, spawn a capability-backed process, use a secret, contact the network, or
//! reserve resources. Runtime crates should convert their concrete operations
//! into these variants so policy, approvals, resources, and audit all reason
//! about the same shape. Actions intentionally contain scoped/virtual contract
//! types, never raw host paths or secret values.

use std::net::IpAddr;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    ApprovalRequest, CapabilityId, CapabilitySet, EffectKind, ExtensionId, HostApiError, MountView,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct NetworkTarget {
    scheme: NetworkScheme,
    host: String,
    port: Option<u16>,
}

impl NetworkTarget {
    pub fn new(
        scheme: NetworkScheme,
        host: impl Into<String>,
        port: Option<u16>,
    ) -> Result<Self, HostApiError> {
        let host = validate_exact_host(host.into())?;
        Ok(Self { scheme, host, port })
    }

    pub fn scheme(&self) -> NetworkScheme {
        self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }
}

impl<'de> Deserialize<'de> for NetworkTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawNetworkTarget {
            scheme: NetworkScheme,
            host: String,
            port: Option<u16>,
        }

        let raw = RawNetworkTarget::deserialize(deserializer)?;
        Self::new(raw.scheme, raw.host, raw.port).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct NetworkTargetPattern {
    scheme: Option<NetworkScheme>,
    host_pattern: String,
    port: Option<u16>,
}

impl NetworkTargetPattern {
    pub fn new(
        scheme: Option<NetworkScheme>,
        host_pattern: impl Into<String>,
        port: Option<u16>,
    ) -> Result<Self, HostApiError> {
        let host_pattern = validate_host_pattern(host_pattern.into())?;
        Ok(Self {
            scheme,
            host_pattern,
            port,
        })
    }

    pub fn scheme(&self) -> Option<NetworkScheme> {
        self.scheme
    }

    pub fn host_pattern(&self) -> &str {
        &self.host_pattern
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }
}

impl<'de> Deserialize<'de> for NetworkTargetPattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawNetworkTargetPattern {
            scheme: Option<NetworkScheme>,
            host_pattern: String,
            port: Option<u16>,
        }

        let raw = RawNetworkTargetPattern::deserialize(deserializer)?;
        Self::new(raw.scheme, raw.host_pattern, raw.port).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub allowed_targets: Vec<NetworkTargetPattern>,
    pub deny_private_ip_ranges: bool,
    pub max_egress_bytes: Option<u64>,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            allowed_targets: Vec::new(),
            deny_private_ip_ranges: true,
            max_egress_bytes: None,
        }
    }
}

fn validate_exact_host(host: String) -> Result<String, HostApiError> {
    let host = normalize_host(host)?;
    if host.contains('*') {
        return Err(HostApiError::invalid_network_target(
            host,
            "wildcards are only allowed in network target patterns",
        ));
    }
    validate_safe_host(&host)?;
    Ok(host)
}

fn validate_host_pattern(host_pattern: String) -> Result<String, HostApiError> {
    let host_pattern = normalize_host(host_pattern)?;
    if let Some(suffix) = host_pattern.strip_prefix("*.") {
        if suffix.contains('*') {
            return Err(HostApiError::invalid_network_target(
                host_pattern,
                "only one leading wildcard label is allowed",
            ));
        }
        validate_safe_host(suffix)?;
        return Ok(format!("*.{suffix}"));
    }
    if host_pattern.contains('*') {
        return Err(HostApiError::invalid_network_target(
            host_pattern,
            "wildcards must be a single leading '*.' label",
        ));
    }
    validate_safe_host(&host_pattern)?;
    Ok(host_pattern)
}

fn normalize_host(host: String) -> Result<String, HostApiError> {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return Err(HostApiError::invalid_network_target(
            host,
            "host must not be empty",
        ));
    }
    if trimmed.len() > 253 {
        return Err(HostApiError::invalid_network_target(
            host,
            "host must be at most 253 bytes",
        ));
    }
    if trimmed.contains("://") || trimmed.contains('/') || trimmed.contains('\\') {
        return Err(HostApiError::invalid_network_target(
            host,
            "host must not include scheme, path, or path separators",
        ));
    }
    if trimmed
        .chars()
        .any(|c| c == '\0' || c.is_control() || c.is_whitespace())
    {
        return Err(HostApiError::invalid_network_target(
            host,
            "host must not contain NUL, control characters, or whitespace",
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn validate_safe_host(host: &str) -> Result<(), HostApiError> {
    if host == "localhost" || host.ends_with(".localhost") {
        return Err(HostApiError::invalid_network_target(
            host,
            "localhost targets are denied by default",
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public_ip(ip) {
            return Err(HostApiError::invalid_network_target(
                host,
                "private, loopback, link-local, multicast, or unspecified IPs are denied by default",
            ));
        }
        return Ok(());
    }
    validate_dns_name(host)
}

fn validate_dns_name(host: &str) -> Result<(), HostApiError> {
    for label in host.split('.') {
        if label.is_empty() {
            return Err(HostApiError::invalid_network_target(
                host,
                "empty DNS labels are not allowed",
            ));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(HostApiError::invalid_network_target(
                host,
                "DNS labels must not start or end with '-'",
            ));
        }
        if label
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
        {
            return Err(HostApiError::invalid_network_target(
                host,
                "only ASCII letters, digits, '-' and '.' are allowed in hosts",
            ));
        }
    }
    Ok(())
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified())
        }
        IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
    }
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
